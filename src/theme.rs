//! The app's colors, and the terminal's palette.

use ghostty_vt::style::{Palette, RgbColor};
use gpui::{Hsla, Rgba, rgb};

const BACKGROUND: u32 = 0x1e1e1e;
const FOREGROUND: u32 = 0xd4d4d4;

/// The 16 ANSI colors: normal, then bright. The rest of the 256 are libghostty's.
const ANSI: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b,
    0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];

pub fn background() -> Hsla {
    rgb(BACKGROUND).into()
}

pub fn foreground() -> Hsla {
    rgb(FOREGROUND).into()
}

pub fn titlebar() -> Hsla {
    rgb(0x141416).into()
}

pub fn tab_active() -> Hsla {
    rgb(0x1e1e1e).into()
}

pub fn tab_hover() -> Hsla {
    rgb(0x26262a).into()
}

pub fn border() -> Hsla {
    rgb(0x2c2c30).into()
}

pub fn muted() -> Hsla {
    rgb(0x94a3b8).into()
}

pub fn error() -> Hsla {
    rgb(0xf87171).into()
}

pub fn close_hover() -> Hsla {
    rgb(0xc42b1c).into()
}

/// The terminal's default foreground and background.
pub fn terminal_colors() -> (RgbColor, RgbColor) {
    (to_rgb(FOREGROUND), to_rgb(BACKGROUND))
}

/// The terminal's palette: libghostty's, with the app's ANSI colors.
pub fn palette(default: Palette) -> Palette {
    let mut palette = default;
    for (slot, color) in palette.0.iter_mut().zip(ANSI) {
        *slot = to_rgb(color);
    }
    palette
}

pub fn to_hsla(c: RgbColor) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.,
        g: f32::from(c.g) / 255.,
        b: f32::from(c.b) / 255.,
        a: 1.,
    }
    .into()
}

fn to_rgb(c: u32) -> RgbColor {
    let [_, r, g, b] = c.to_be_bytes();
    RgbColor { r, g, b }
}
