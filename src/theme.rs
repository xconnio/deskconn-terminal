//! The app's colors, and the terminal's palette.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::{Hsla, Rgba, rgb};

pub fn background() -> Hsla {
    rgb(0x1e1e1e).into()
}

pub fn foreground() -> Hsla {
    rgb(0xd4d4d4).into()
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

/// The 16 ANSI colors: normal, then bright.
const ANSI: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666, 0xf14c4c, 0x23d18b,
    0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];

/// The color a cell's color is, given the colors the terminal's programs have set.
pub fn resolve(color: Color, colors: &Colors) -> Hsla {
    match color {
        Color::Spec(c) => to_hsla(c),
        Color::Indexed(i) => colors[i as usize].map_or_else(|| indexed(i), to_hsla),
        Color::Named(name) => colors[name].map_or_else(|| named(name), to_hsla),
    }
}

/// A named color's bright variant, which bold text of a normal color is drawn in.
pub fn brighten(color: Color) -> Color {
    match color {
        Color::Named(name) if (name as usize) < 8 => Color::Indexed(name as u8 + 8),
        Color::Indexed(i) if i < 8 => Color::Indexed(i + 8),
        color => color,
    }
}

fn named(name: NamedColor) -> Hsla {
    match name {
        NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => foreground(),
        NamedColor::Background => background(),
        NamedColor::DimForeground => foreground().opacity(0.66),
        name => {
            let i = name as usize;
            if i < 16 {
                indexed(i as u8)
            } else {
                // DimBlack to DimWhite.
                indexed((i - NamedColor::DimBlack as usize) as u8).opacity(0.66)
            }
        }
    }
}

/// One of the 256 indexed colors: the ANSI colors, a 6×6×6 cube, and 24 grays.
fn indexed(i: u8) -> Hsla {
    match i {
        0..16 => rgb(ANSI[i as usize]).into(),
        16..232 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            to_hsla(Rgb {
                r: level(i / 36),
                g: level(i / 6 % 6),
                b: level(i % 6),
            })
        }
        232.. => {
            let v = 8 + 10 * (i - 232);
            to_hsla(Rgb { r: v, g: v, b: v })
        }
    }
}

fn to_hsla(c: Rgb) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.,
        g: f32::from(c.g) / 255.,
        b: f32::from(c.b) / 255.,
        a: 1.,
    }
    .into()
}

/// The color of a cell's color as RGB, which programs may ask for.
pub fn to_rgb(color: Hsla) -> Rgb {
    let c = Rgba::from(color);
    let byte = |v: f32| (v * 255.).round() as u8;
    Rgb {
        r: byte(c.r),
        g: byte(c.g),
        b: byte(c.b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays() {
        assert_eq!(to_rgb(indexed(16)), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(to_rgb(indexed(231)), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(to_rgb(indexed(196)), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(to_rgb(indexed(232)), Rgb { r: 8, g: 8, b: 8 });
    }
}
