use std::fs;

use anyhow;
use directories::BaseDirs;
use gpt::{Session, Thread};
use rusqlite::Connection;
use std::io;
use std::path::PathBuf;
use uuid::Uuid;

const SCHEMA_CMD: &str = r#"
    CREATE TABLE thread(id VARCHAR PRIMARY KEY);

    CREATE TABLE message(
      thread_id VARCHAR,
      role INTEGER,
      content VARCHAR,
      timestamp INTEGER,
      FOREIGN KEY (thread_id) REFERENCES thread (id)
    );

"#;

pub fn data_dir() -> io::Result<PathBuf> {
    let dir = BaseDirs::new()
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not locate the home directory",
        ))?
        .data_dir()
        .to_path_buf()
        .join("gpt_rs");

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
    fn from_db(conn: &Connection, id: Uuid) -> Result<Self, rusqlite::Error>;
    fn to_db(&self, conn: &Connection) -> Result<(), rusqlite::Error>;
}

impl DbStore for Thread {
    fn to_db(&self, conn: &Connection) -> Result<(), rusqlite::Error> {
        todo!();
    }

    fn from_db(conn: &Connection, id: Uuid) -> Result<Self, rusqlite::Error> {
        todo!();
    }
}
