use crate::data_dir;
use anyhow;

use rusqlite::{self, Connection};

mod schema {
    pub const SQL: &str = include_str!(concat!(env!("OUT_DIR"), "/schema.sql"));
}

pub const DB_FILENAME: &str = "gpt.db";

pub fn init_db() -> anyhow::Result<Connection> {
    let db_path = data_dir!()?.join("gpt.db");
    let conn = Connection::open(db_path)?;
    conn.execute_batch(schema::SQL)?;
    Ok(conn)
}

#[cfg(test)]
mod test_db {
    use super::*;

    #[test]
    fn test_init_db() {
        let _ = init_db().unwrap();
    }
}
