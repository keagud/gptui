use std::borrow::Cow;

use arboard::Clipboard;

#[cfg(target_os = "linux")]
mod linux_no_de {


    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Stdio;
    use std::str::FromStr;
    use uuid::Uuid;
    use which::which;

    // a workaround for setups that use x11 and a window manager, but no desktop environment
    // I don't use wayland, so a PR with wayland support would be much appreciated
    pub(super) fn select_xclip(text: &str) -> crate::Result<bool> {
        if which("xclip").is_ok() {
            let clip_file = PathBuf::from_str("/tmp")
                .map_err(|e| crate::Error::Other(e.into()))?
                .join(Uuid::new_v4().as_simple().to_string())
                .with_extension("clip");

            fs::write(&clip_file, text.as_bytes())?;

            Command::new("xclip")
                .args(["-selection", "c"])
                .arg(&clip_file)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;

            let _ = fs::remove_file(&clip_file);

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub fn copy<'a, T>(text: T) -> crate::Result<()>
where
    T: Into<Cow<'a, str>>,
{
    let text: Cow<'_, str> = text.into();

    #[cfg(target_os = "linux")]
    if linux_no_de::select_xclip(text.as_ref())? {
        return Ok(());
    }

    Clipboard::new()?.set_text(text)?;

    Ok(())
}

pub fn paste() -> Result<String, arboard::Error> {
    let res = Clipboard::new()?.get_text()?;

    Ok(res)
}
