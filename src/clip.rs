use std::borrow::Cow;

use arboard::Clipboard;

pub fn copy<'a, T>(text: T) -> Result<(), arboard::Error>
where
    T: Into<Cow<'a, str>>,
{
    Clipboard::new()?.set_text(text)?;

    Ok(())
}

pub fn paste() -> Result<String, arboard::Error> {
    let res = Clipboard::new()?.get_text()?;

    Ok(res)
}
