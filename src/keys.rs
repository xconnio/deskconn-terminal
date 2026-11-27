//! What the terminal sends for a key, encoded by libghostty-vt as the terminal's modes ask:
//! legacy xterm sequences, or the Kitty keyboard protocol.

use ghostty_vt::Terminal;
use ghostty_vt::key::{Action, Encoder, Event, Key, Mods};
use gpui::Keystroke;

/// Encodes the user's keys for a terminal.
pub struct KeyEncoder {
    encoder: Encoder,
    event: Event,
}

impl KeyEncoder {
    pub fn new() -> ghostty_vt::error::Result<Self> {
        Ok(Self {
            encoder: Encoder::new()?,
            event: Event::new()?,
        })
    }

    /// The bytes keystroke sends to the shell of terminal; empty if it sends nothing.
    pub fn encode(&mut self, keystroke: &Keystroke, terminal: &Terminal) -> Vec<u8> {
        let m = &keystroke.modifiers;
        let mut mods = Mods::empty();
        mods.set(Mods::SHIFT, m.shift);
        mods.set(Mods::ALT, m.alt);
        mods.set(Mods::CTRL, m.control);
        mods.set(Mods::SUPER, m.platform);

        // The text the key types, unless it's a control character: the encoder derives
        // those from the key and the modifiers.
        let text = keystroke
            .key_char
            .as_deref()
            .filter(|text| !text.chars().any(char::is_control));
        let unshifted = match keystroke.key.chars().collect::<Vec<_>>()[..] {
            [c] => c,
            _ => '\0',
        };
        self.event
            .set_action(Action::Press)
            .set_key(key(&keystroke.key))
            .set_mods(mods)
            .set_utf8(text)
            .set_unshifted_codepoint(unshifted);

        let mut bytes = Vec::new();
        self.encoder.set_options_from_terminal(terminal);
        if self.encoder.encode_to_vec(&self.event, &mut bytes).is_err() {
            bytes.clear();
        }
        bytes
    }
}

/// The physical key GPUI names key, as a US layout has it.
fn key(key: &str) -> Key {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Digit0,
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
        Key::Digit7,
        Key::Digit8,
        Key::Digit9,
    ];
    const FUNCTION: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];

    match key {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "`" => Key::Backquote,
        "\\" => Key::Backslash,
        "[" => Key::BracketLeft,
        "]" => Key::BracketRight,
        "," => Key::Comma,
        "=" => Key::Equal,
        "-" => Key::Minus,
        "." => Key::Period,
        "'" => Key::Quote,
        ";" => Key::Semicolon,
        "/" => Key::Slash,
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c @ 'a'..='z'), None) => LETTERS[c as usize - 'a' as usize],
                (Some(c @ '0'..='9'), None) => DIGITS[c as usize - '0' as usize],
                (Some('f'), Some(_)) => key[1..]
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| FUNCTION.get(n.wrapping_sub(1)).copied())
                    .unwrap_or(Key::Unidentified),
                _ => Key::Unidentified,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_vt::terminal::Options;

    fn esc(terminal: &Terminal, key: &str) -> Vec<u8> {
        let mut keystroke = Keystroke::parse(key).unwrap();
        if keystroke.key.len() == 1 && !keystroke.modifiers.control && !keystroke.modifiers.alt {
            keystroke.key_char = Some(keystroke.key.clone());
        }
        KeyEncoder::new().unwrap().encode(&keystroke, terminal)
    }

    #[test]
    fn keys() {
        let mut terminal = Terminal::new(Options::default()).unwrap();
        assert_eq!(esc(&terminal, "a"), b"a");
        assert_eq!(esc(&terminal, "ctrl-c"), b"\x03");
        assert_eq!(esc(&terminal, "alt-b"), b"\x1bb");
        assert_eq!(esc(&terminal, "enter"), b"\r");
        assert_eq!(esc(&terminal, "backspace"), b"\x7f");
        assert_eq!(esc(&terminal, "up"), b"\x1b[A");
        assert_eq!(esc(&terminal, "ctrl-right"), b"\x1b[1;5C");
        assert_eq!(esc(&terminal, "shift-tab"), b"\x1b[Z");
        assert_eq!(esc(&terminal, "pageup"), b"\x1b[5~");
        assert_eq!(esc(&terminal, "f5"), b"\x1b[15~");
        // Application cursor keys, as full-screen programs turn on.
        terminal.vt_write(b"\x1b[?1h");
        assert_eq!(esc(&terminal, "up"), b"\x1bOA");
    }
}
