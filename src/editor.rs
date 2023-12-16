use directories::BaseDirs;
use edit::get_editor;
use std::env::temp_dir;
use std::io::{ErrorKind, Read};
use std::process::{Command, Stdio};
use std::{fs, io};
use tempfile::tempfile;

fn editor_binary() -> anyhow::Result<String> {
    #[cfg(target_family = "windows")]
    let editor = get_editor().map(|s| s.to_string_lossy().into())?;

    #[cfg(target_family = "unix")]
    let editor =
        std::env::var("EDITOR").or_else(|_| get_editor().map(|s| s.to_string_lossy().into()))?;

    #[cfg(not(any(target_family = "unix", target_family = "windows")))]
    compile_error!("Unsupported compile target");

    Ok(editor)
}

/// Open a new temporary file in an external editor.
/// When the editor is closed, if the file has any non-whitespace content,
/// return Ok(Some(content)).
/// Otherwise return Ok(None).
pub fn input_from_editor() -> anyhow::Result<Option<String>> {
    let temp_filename = uuid::Uuid::new_v4().simple().to_string();
    let editor = editor_binary()?;

    let temp_filepath = temp_dir()
        .join(temp_filename)
        .with_extension("txt")
        .to_string_lossy()
        .to_string();

    Command::new(editor)
        .arg(&temp_filepath)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .status()?;

    let file_contents = match fs::read_to_string(temp_filepath) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => Err(e),
    }?;

    Ok(match file_contents.trim() {
        s if s.is_empty() => None,
        s => Some(s.into()),
    })
}

#[cfg(test)]
mod test_editor {
    use super::*;

    #[test]
    fn test_find_editor() {
        dbg!(editor_binary().unwrap());
    }
}
