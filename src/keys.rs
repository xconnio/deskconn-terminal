//! What the terminal sends for a key, as xterm does.

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// The bytes a keystroke sends to the shell, or None if it sends nothing.
pub fn to_esc(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let mods = &keystroke.modifiers;
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let modifier = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.control);
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // Cursor keys and Home/End: ESC O x in application mode, ESC [ 1;m x when modified.
    let cursor = |c: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    // Keys sent as ESC [ n ~, or ESC [ n;m ~ when modified.
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~")
        } else {
            format!("\x1b[{n}~")
        }
        .into_bytes()
    };
    // F1 to F4: ESC O x, or ESC [ 1;m x when modified.
    let ss3 = |c: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{c}")
        } else {
            format!("\x1bO{c}")
        }
        .into_bytes()
    };

    let bytes = match keystroke.key.as_str() {
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" => alt_prefixed(mods.alt, b"\r"),
        "tab" if mods.shift => b"\x1b[Z".to_vec(),
        "tab" => alt_prefixed(mods.alt, b"\t"),
        "escape" => alt_prefixed(mods.alt, b"\x1b"),
        "backspace" if mods.control => alt_prefixed(mods.alt, b"\x08"),
        "backspace" => alt_prefixed(mods.alt, b"\x7f"),
        "space" if mods.control => alt_prefixed(mods.alt, b"\0"),
        key if mods.control => alt_prefixed(mods.alt, &[control_char(key)?]),
        key => {
            // Alt+key may type nothing of its own: it's Meta and the key.
            let text = match keystroke.key_char.as_deref() {
                Some(text) => text,
                None if mods.alt && key.chars().count() == 1 => key,
                None => return None,
            };
            alt_prefixed(mods.alt && !mods.platform, text.as_bytes())
        }
    };
    Some(bytes)
}

/// The control character Ctrl sends with key, such as 0x03 for Ctrl+C.
fn control_char(key: &str) -> Option<u8> {
    let [c] = key.as_bytes() else { return None };
    match c {
        b'a'..=b'z' => Some(c - b'a' + 1),
        b'@' | b'2' => Some(0),
        b'[' | b'3' => Some(0x1b),
        b'\\' | b'4' => Some(0x1c),
        b']' | b'5' => Some(0x1d),
        b'^' | b'6' => Some(0x1e),
        b'_' | b'-' | b'7' => Some(0x1f),
        b'?' | b'8' => Some(0x7f),
        _ => None,
    }
}

/// bytes, after ESC if Alt is held, as Alt is Meta in a terminal.
fn alt_prefixed(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(key: &str) -> Vec<u8> {
        to_esc(&Keystroke::parse(key).unwrap(), TermMode::empty()).unwrap()
    }

    #[test]
    fn keys() {
        assert_eq!(esc("ctrl-c"), b"\x03");
        assert_eq!(esc("alt-b"), b"\x1bb");
        assert_eq!(esc("up"), b"\x1b[A");
        assert_eq!(esc("ctrl-right"), b"\x1b[1;5C");
        assert_eq!(esc("shift-tab"), b"\x1b[Z");
        assert_eq!(esc("pageup"), b"\x1b[5~");
        assert_eq!(esc("f5"), b"\x1b[15~");
        assert_eq!(esc("backspace"), b"\x7f");
        let up = Keystroke::parse("up").unwrap();
        assert_eq!(to_esc(&up, TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
    }
}
