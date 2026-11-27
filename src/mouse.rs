//! What the terminal sends for the mouse to a program that asked for it, as Claude Code, Codex,
//! vim, and htop do: encoded by libghostty-vt in the format the program set.

use ghostty_vt::Terminal;
use ghostty_vt::key::Mods;
use ghostty_vt::mouse::{Action, Button, Encoder, EncoderSize, Event, Position};
use gpui::{Modifiers, MouseButton, Pixels, Point, Size};

/// Encodes the user's mouse for a terminal.
pub struct MouseEncoder {
    encoder: Encoder,
    event: Event,
}

impl MouseEncoder {
    pub fn new() -> ghostty_vt::error::Result<Self> {
        Ok(Self {
            encoder: Encoder::new()?,
            event: Event::new()?,
        })
    }

    /// The bytes for a mouse event at position, in cells from the grid's origin, with cells of
    /// cell; empty if the program doesn't take it, as it may not take motion. Motion with a button
    /// is a drag, with the button held.
    pub fn encode(
        &mut self,
        terminal: &Terminal,
        action: Action,
        button: Option<Button>,
        modifiers: Modifiers,
        position: Point<f32>,
        cell: Size<Pixels>,
    ) -> Vec<u8> {
        let (Ok(cols), Ok(rows)) = (terminal.cols(), terminal.rows()) else {
            return Vec::new();
        };
        // Whole pixels, as the encoder has them: a position finds its cell by them, so it's given
        // in cells of the rounded size.
        let width = (f32::from(cell.width).round() as u32).max(1);
        let height = (f32::from(cell.height).round() as u32).max(1);
        self.encoder
            .set_options_from_terminal(terminal)
            .set_size(EncoderSize {
                screen_width: width * u32::from(cols),
                screen_height: height * u32::from(rows),
                cell_width: width,
                cell_height: height,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            })
            .set_any_button_pressed(action == Action::Motion && button.is_some())
            .set_track_last_cell(true);

        let mut mods = Mods::empty();
        mods.set(Mods::SHIFT, modifiers.shift);
        mods.set(Mods::ALT, modifiers.alt);
        mods.set(Mods::CTRL, modifiers.control);
        mods.set(Mods::SUPER, modifiers.platform);
        self.event
            .set_action(action)
            .set_button(button)
            .set_mods(mods)
            .set_position(Position {
                x: position.x * width as f32,
                y: position.y * height as f32,
            });

        let mut bytes = Vec::new();
        if self.encoder.encode_to_vec(&self.event, &mut bytes).is_err() {
            bytes.clear();
        }
        bytes
    }
}

/// The button GPUI names button, as terminals number them.
pub fn button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Middle => Some(Button::Middle),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Navigate(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_vt::terminal::Options;
    use gpui::{point, px, size};

    #[test]
    fn reports() {
        let mut terminal = Terminal::new(Options::default()).unwrap();
        let mut mouse = MouseEncoder::new().unwrap();
        let cell = size(px(8.4), px(18.));
        let mut encode = |terminal: &Terminal, action, button| {
            mouse.encode(terminal, action, button, Modifiers::default(), point(2.5, 1.2), cell)
        };
        // Nothing until a program asks for the mouse.
        assert!(encode(&terminal, Action::Press, Some(Button::Left)).is_empty());
        // Normal tracking, in the SGR format.
        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(encode(&terminal, Action::Press, Some(Button::Left)), b"\x1b[<0;3;2M");
        assert_eq!(encode(&terminal, Action::Release, Some(Button::Left)), b"\x1b[<0;3;2m");
        // The wheel, up.
        assert_eq!(encode(&terminal, Action::Press, Some(Button::Four)), b"\x1b[<64;3;2M");
    }
}
