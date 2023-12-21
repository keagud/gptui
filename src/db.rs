use crate::config::CONFIG;
use crate::session::{Message, Role, Thread};

use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};

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

    CREATE TABLE title(
      id VARCHAR PRIMARY KEY,
      content TEXT
    );

"#;

/// Create tables
fn setup_table_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_CMD)
}

pub fn init_db() -> anyhow::Result<Connection> {
    let db_path = CONFIG.data_dir().join("gpt.db");

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

        if let Some(title) = self.thread_title() {
            conn.execute(
                "INSERT OR IGNORE INTO title (id, content) VALUES (?1, ?2)",
                [&self.str_id(), title],
            )?;
        }

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

        let title = conn
            .prepare("SELECT content FROM title WHERE id = ?1")?
            .query_row([&id_str], |row| row.get::<_, String>(0))
            .optional()?;

        let mut new_thread = Thread::new(messages, &model, id);

        if let Some(ref title) = title {
            new_thread.set_title(title);
        }

        Ok(new_thread)
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
