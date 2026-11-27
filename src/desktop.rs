//! Registers the app with a Linux desktop: an entry and an icon in the user's data directory,
//! as the desktop finds the window's icon by its app ID. This also lists the app among the others,
//! to start from there.

use std::path::Path;

use anyhow::{Context, Result};

use crate::APP_ID;

const ICON: &[u8] = include_bytes!("../assets/icon.png");

/// Writes the desktop entry, to start this very executable, and the icon, unless they're there.
pub fn register() -> Result<()> {
    let data = dirs::data_dir().context("no data directory")?;
    let exe = std::env::current_exe().context("find the executable")?;
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=DeskConn Terminal\n\
         Comment=Terminals on your machines\n\
         Exec={}\n\
         Icon={APP_ID}\n\
         StartupWMClass={APP_ID}\n\
         Categories=System;TerminalEmulator;\n",
        exec_quote(&exe.to_string_lossy())
    );
    write_if_changed(
        &data.join("applications").join(format!("{APP_ID}.desktop")),
        entry.as_bytes(),
    )?;
    write_if_changed(
        &data.join("icons/hicolor/512x512/apps").join(format!("{APP_ID}.png")),
        ICON,
    )
}

/// Quotes path for a desktop entry's Exec key: in double quotes, with the characters special
/// there escaped, and the backslashes of those escapes escaped again, as the key's a string.
fn exec_quote(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push_str("\\\\");
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    let dir = path.parent().expect("the file is in a directory");
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(
            exec_quote("/home/me/deskconn-terminal"),
            r#""/home/me/deskconn-terminal""#
        );
        assert_eq!(exec_quote("/home/me/my $apps/x"), r#""/home/me/my \\$apps/x""#);
    }
}
