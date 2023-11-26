use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    let out_dir_env = env::var_os("OUT_DIR").unwrap();
    let out_file_path = Path::new(&out_dir_env).join("schema.sql");

    let sql_content = include_str!("./schema/schema.sql");

    fs::File::create(out_file_path)
        .unwrap()
        .write_all(sql_content.as_bytes())
        .unwrap();
}
