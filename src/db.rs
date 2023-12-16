use std::fs;

use crate::session::{Message, Role, Thread};

use directories::BaseDirs;
use rusqlite::{params, Connection};
use std::io;
use std::path::PathBuf;
use uuid::Uuid;

const SCHEMA_CMD: &str = r#"
    CREATE TABLE thread(
        id VARCHAR PRIMARY KEY,
        model VARCHAR
    );

    CREATE TABLE message(
      thread_id VARCHAR,
      role INTEGER,
      content VARCHAR,
      timestamp FLOAT,
      FOREIGN KEY (thread_id) REFERENCES thread (id)
    );

"#;

pub fn data_dir() -> io::Result<PathBuf> {
    let dir = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_assets")
    } else {
        BaseDirs::new()
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                "Could not locate the home directory",
            ))?
            .data_dir()
            .to_path_buf()
            .join("gpt_rs")
    };

    match dir.try_exists() {
        Ok(true) => Ok(dir),
        Ok(false) => {
            fs::create_dir_all(&dir)?;
            Ok(dir)
        }
        Err(e) => Err(e),
    }
}

/// Create tables
fn setup_table_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_CMD)
}

pub fn init_db() -> anyhow::Result<Connection> {
    let db_path = data_dir()?.join("gpt.db");

    let requires_init = !db_path.try_exists()?;
    let conn = Connection::open(&db_path)?;

    if requires_init {
        setup_table_schema(&conn)?;
    }

    Ok(conn)
}

pub trait DbStore: Sized {
    fn from_db(conn: &Connection, id: Uuid) -> anyhow::Result<Self>;
    fn to_db(&self, conn: &mut Connection) -> anyhow::Result<()>;
    fn get_all(conn: &mut Connection) -> anyhow::Result<Vec<Self>>;
}

impl DbStore for Thread {
    fn to_db(&self, conn: &mut Connection) -> anyhow::Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO thread (id, model) VALUES (?1, ?2)",
            [&self.str_id(), &self.model],
        )?;

        // get the most recent message in the db for this thread
        //
        let last_ts_result: rusqlite::Result<f64> = conn
            .prepare(
                r#"
            SELECT timestamp FROM message 
            WHERE thread_id = ?1
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
            )?
            .query_row([&self.str_id()], |row| row.get(0));

        let messages_to_store: Vec<&Message> = match last_ts_result {
            Ok(n) => Ok(self
                .messages()
                .into_iter()
                .filter(|m| m.timestamp_epoch() > n)
                .collect()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(self.messages()),
            Err(e) => Err(e),
        }?;

        let tx = conn.transaction()?;

        {
            let mut tx_stmt = tx.prepare(
            r#"INSERT INTO message (thread_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)"#,
        )?;

            for message in messages_to_store {
                tx_stmt.execute(params![
                    &self.str_id(),
                    message.role.to_num(),
                    &message.content,
                    message.timestamp_epoch(),
                ])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    fn from_db(conn: &Connection, id: Uuid) -> anyhow::Result<Self> {
        let id_str = id.as_simple().to_string();

        let model: String = conn
            .prepare(r" SELECT model FROM thread WHERE id = ?1 ")?
            .query_row([&id_str], |row| row.get(0))?;

        let mut stmt = conn.prepare(
            r#"
          
          SELECT role, content, timestamp
          FROM message
          WHERE thread_id = ?1
          ORDER BY timestamp ASC

        "#,
        )?;

        let messages: Vec<Message> = stmt
            .query_and_then([&id_str], |row| -> anyhow::Result<Message> {
                Ok(Message::new_from_db(
                    Role::from_num(row.get::<usize, i64>(0)?.try_into()?)?,
                    row.get(1)?,
                    row.get(2)?,
                ))
            })?
            .collect::<anyhow::Result<Vec<Message>>>()?;

        Ok(Thread::new(messages, &model, id))
    }

    fn get_all(conn: &mut Connection) -> anyhow::Result<Vec<Self>> {
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM thread")?
            .query_and_then([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;

        ids.into_iter()
            .map(|id| Self::from_db(conn, Uuid::parse_str(&id)?))
            .collect()
    }
}
