use crate::config::CONFIG;
use crate::llm::LlmModel;
use crate::session::{Message, Role, Summary, Thread};

use itertools::Itertools;
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};

use uuid::Uuid;

const SCHEMA_CMD: &str = include_str!(concat!(env!("OUT_DIR"), "/init.sql"));

// const SCHEMA_CMD: &str = r#"
//     CREATE TABLE thread(
//         id VARCHAR PRIMARY KEY,
//         model VARCHAR
//     );

//     CREATE TABLE message(
//       thread_id VARCHAR,
//       role INTEGER,
//       content VARCHAR,
//       timestamp FLOAT,
//       tokens INTEGER,
//       FOREIGN KEY (thread_id) REFERENCES thread (id)
//     );

//     CREATE TABLE title(
//       id VARCHAR PRIMARY KEY,
//       content TEXT
//     );

// "#;

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
    type Error;
    type Key: Sized;
    fn from_db(conn: &Connection, id: Self::Key) -> Result<Self, Self::Error>;
    fn to_db(&self, conn: &mut Connection) -> Result<(), Self::Error>;
    fn get_all(conn: &mut Connection) -> Result<Vec<Self>, Self::Error>;
    fn drop_from_db(&self, conn: &mut Connection) -> Result<bool, Self::Error>;
}

impl DbStore for Thread {
    type Error = crate::Error;
    type Key = Uuid;
    fn to_db(&self, conn: &mut Connection) -> Result<(), Self::Error> {
        conn.execute(
            "INSERT OR IGNORE INTO thread (id, model) VALUES (?1, ?2)",
            [&self.str_id(), &self.model.to_string()],
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
            let mut msg_update = tx.prepare(
            r#"INSERT INTO message (thread_id, role, content, timestamp, tokens) VALUES (?1, ?2, ?3, ?4, ?5)"#,
        )?;

            for message in messages_to_store {
                msg_update.execute(params![
                    &self.str_id(),
                    message.role.to_num(),
                    &message.content,
                    message.timestamp_epoch(),
                    message.token_count
                ])?;
            }
        }

        {
            let mut summary_update = tx.prepare(r#"INSERT OR IGNORE INTO summary (thread_id, start_index, end_index, content) VALUES (?1, ?2, ?3, ?4)"#)?;

            for summary in self.summary_entries.iter() {
                summary_update.execute(params![
                    &self.str_id(),
                    summary.start_index,
                    summary.end_index,
                    &summary.content
                ])?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    fn from_db(conn: &Connection, id: Self::Key) -> Result<Self, Self::Error> {
        let id_str = id.as_simple().to_string();

        let model_label: String = conn
            .prepare(r" SELECT model FROM thread WHERE id = ?1 ")?
            .query_row([&id_str], |row| row.get(0))?;

        let model = LlmModel::from_label(&model_label).ok_or_else(|| {
            crate::Error::DbRetrievalError(
                anyhow::format_err!("{} is not a valid model", &model_label).into(),
            )
        })?;

        let mut stmt = conn.prepare(
            r#"
          
          SELECT role, content, timestamp, tokens
          FROM message
          WHERE thread_id = ?1
          ORDER BY timestamp ASC

        "#,
        )?;

        let messages: Vec<Message> = stmt
            .query_and_then([&id_str], |row| -> rusqlite::Result<Message> {
                Ok(Message::new_from_db(
                    Role::from_num(
                        row.get::<usize, i64>(0)?
                            .try_into()
                            .map_err(|_e| rusqlite::Error::InvalidColumnIndex(0))?,
                    )
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            })?
            .collect::<Result<Vec<Message>, _>>()?;

        let title = conn
            .prepare("SELECT content FROM title WHERE id = ?1")?
            .query_row([&id_str], |row| row.get::<_, String>(0))
            .optional()?;

        // load any stored summary data
        let summaries = conn
            .prepare(
                r#" SELECT start_index, end_index, content FROM summary
              WHERE thread_id = ?1 "#,
            )?
            .query_and_then(params![&id_str], |row| -> rusqlite::Result<Summary> {
                Ok(Summary::new(
                    id,
                    row.get::<usize, usize>(0)?,
                    row.get::<usize, usize>(1)?,
                    &row.get::<usize, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut new_thread = Thread::new(messages, model, id);

        new_thread.summary_entries = summaries;

        if let Some(ref title) = title {
            new_thread.set_title(title);
        }

        Ok(new_thread)
    }

    fn drop_from_db(&self, conn: &mut Connection) -> Result<bool, Self::Error> {
        // clear associated messages
        conn.prepare("DELETE FROM message WHERE thread_id = ?1")?
            .execute(params![&self.str_id()])?;

        //clear stored title if it exists
        conn.prepare("DELETE FROM title WHERE id = ?1")?
            .execute(params![&self.str_id()])?;

        // clear associated summary data
        conn.prepare("DELETE FROM summary WHERE thread_id = 1?")?
            .execute(params![&self.str_id()])?;

        // delete the thread itself
        let altered_rows_count = conn
            .prepare("DELETE FROM thread WHERE id = ?1")?
            .execute(params![&self.str_id()])?;

        Ok(altered_rows_count > 0)
    }

    fn get_all(conn: &mut Connection) -> Result<Vec<Self>, Self::Error> {
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM thread")?
            .query_and_then([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;

        ids.into_iter()
            .map(|id| {
                Self::from_db(
                    conn,
                    Uuid::parse_str(&id).map_err(|e| crate::Error::DbRetrievalError(e.into()))?,
                )
            })
            .collect()
    }
}
