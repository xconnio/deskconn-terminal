//! Selecting the terminal's text with the mouse, through libghostty-vt's selection gestures, and
//! finding the link under the mouse.

use std::time::{Duration, Instant};

use ghostty_vt::Terminal;
use ghostty_vt::error::Result;
use ghostty_vt::selection::gesture::{
    Autoscroll, AutoscrollTickEvent, DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent,
};
use ghostty_vt::selection::{FormatOptions, SelectWordOptions};
use ghostty_vt::terminal::{Point, PointCoordinate, PointSpace};

use crate::links;

/// libghostty's gestures measure cells in whole pixels, but cells are a fraction of a pixel wide;
/// so positions are given to them in units of this fraction of a cell.
pub const UNIT: f64 = 64.;
/// How soon a click must follow the last to count with it, as a double or triple click.
const REPEAT_INTERVAL: Duration = Duration::from_millis(500);

/// Where the mouse is on the grid: in units of a cell's [UNIT]th from the grid's origin, and the
/// cell it's on, within the grid.
#[derive(Clone, Copy)]
pub struct GridPosition {
    pub x: f64,
    pub y: f64,
    pub cell: PointCoordinate,
}

/// Turns presses and drags into selections, installed as the terminal's.
pub struct Selector {
    gesture: Gesture,
    press: PressEvent,
    drag: DragEvent,
    release: ReleaseEvent,
    tick: AutoscrollTickEvent,
    /// What the time of a click is measured from.
    epoch: Instant,
}

impl Selector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            gesture: Gesture::new()?,
            press: PressEvent::new()?,
            drag: DragEvent::new()?,
            release: ReleaseEvent::new()?,
            tick: AutoscrollTickEvent::new()?,
            epoch: Instant::now(),
        })
    }

    /// Starts selecting at, a word for a double click and a line for a triple click.
    pub fn press(&mut self, term: &Terminal, at: GridPosition) -> Result<()> {
        self.press
            .set_position(at.x, at.y)?
            .set_time(self.epoch.elapsed())?
            .set_repeat_interval(REPEAT_INTERVAL)?
            .set_repeat_distance(UNIT / 2.)?;
        let grid_ref = term.grid_ref(Point::Viewport(at.cell))?;
        let selection = self.press.apply(&mut self.gesture, term, grid_ref)?;
        term.set_selection(selection.as_ref())?;
        Ok(())
    }

    /// Selects to at.
    pub fn drag(&mut self, term: &Terminal, at: GridPosition) -> Result<()> {
        self.drag.set_position(at.x, at.y)?;
        let grid_ref = term.grid_ref(Point::Viewport(at.cell))?;
        if let Some(selection) = self.drag.apply(&mut self.gesture, term, grid_ref, geometry(term)?)? {
            term.set_selection(Some(&selection))?;
        }
        Ok(())
    }

    pub fn release(&mut self, term: &Terminal, at: GridPosition) -> Result<()> {
        let grid_ref = term.grid_ref(Point::Viewport(at.cell)).ok();
        self.release.apply(&mut self.gesture, term, grid_ref)
    }

    /// Whether a drag past the top or bottom of the grid has the view to scroll.
    pub fn autoscrolls(&self, term: &Terminal) -> bool {
        self.gesture
            .autoscroll(term)
            .is_ok_and(|direction| direction != Autoscroll::None)
    }

    /// Scrolls a row the way the drag goes, selecting to at.
    pub fn tick(&mut self, term: &Terminal, at: GridPosition) -> Result<()> {
        self.tick.set_position(at.x, at.y)?;
        if let Some(selection) = self.tick.apply(&mut self.gesture, term, at.cell, geometry(term)?)? {
            term.set_selection(Some(&selection))?;
        }
        Ok(())
    }
}

/// The grid as the gestures see it: cells [UNIT] wide, with no padding.
fn geometry(term: &Terminal) -> Result<Geometry> {
    Ok(Geometry {
        columns: u32::from(term.cols()?),
        cell_width: UNIT as u32,
        padding_left: 0,
        screen_height: u32::from(term.rows()?) * UNIT as u32,
    })
}

/// The selected text, as copying it should give it: wrapped lines joined, and trailing spaces
/// trimmed.
pub fn text(term: &Terminal) -> Option<String> {
    let options = FormatOptions::new().with_unwrap(true).with_trim(true);
    let text = term.format_selection_alloc(options).ok()??;
    Some(String::from_utf8_lossy(&text).into_owned()).filter(|text| !text.is_empty())
}

/// A link on screen, from its first cell to its last, in viewport coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub url: String,
    pub start: PointCoordinate,
    pub end: PointCoordinate,
}

/// The link at cell: one a program made with OSC 8, or a URL in the text.
pub fn link_at(term: &Terminal, cell: PointCoordinate) -> Option<Link> {
    hyperlink_at(term, cell).or_else(|| url_at(term, cell))
}

fn hyperlink(term: &Terminal, cell: PointCoordinate) -> Option<Vec<u8>> {
    let mut uri = [0; 2048];
    let len = term
        .grid_ref(Point::Viewport(cell))
        .ok()?
        .hyperlink_uri(&mut uri)
        .ok()?;
    (len > 0).then(|| uri[..len].to_vec())
}

/// The OSC 8 hyperlink at cell, across the cells of its row that share it.
fn hyperlink_at(term: &Terminal, cell: PointCoordinate) -> Option<Link> {
    let uri = hyperlink(term, cell)?;
    let shares = |x| hyperlink(term, PointCoordinate { x, y: cell.y }).as_ref() == Some(&uri);
    let cols = term.cols().ok()?;
    let (mut start, mut end) = (cell.x, cell.x);
    while start > 0 && shares(start - 1) {
        start -= 1;
    }
    while end + 1 < cols && shares(end + 1) {
        end += 1;
    }
    Some(Link {
        url: String::from_utf8_lossy(&uri).into_owned(),
        start: PointCoordinate { x: start, y: cell.y },
        end: PointCoordinate { x: end, y: cell.y },
    })
}

/// The URL in the text at cell, across wrapped lines.
fn url_at(term: &Terminal, cell: PointCoordinate) -> Option<Link> {
    let grid_ref = term.grid_ref(Point::Viewport(cell)).ok()?;
    let word = term
        .select_word(SelectWordOptions::new(grid_ref).with_boundary_codepoints(links::BOUNDARIES))
        .ok()??;
    let text = term
        .format_selection_alloc(FormatOptions::new().with_selection(&word).with_unwrap(true))
        .ok()??;
    let text = std::str::from_utf8(&text).ok()?;
    let range = links::find(text)?;

    // Cells are counted as characters, which holds for URLs, being ASCII.
    let cols = u64::from(term.cols().ok()?);
    let index = |point: PointCoordinate| u64::from(point.y) * cols + u64::from(point.x);
    let point = |index: u64| PointCoordinate {
        x: (index % cols) as u16,
        y: (index / cols) as u32,
    };
    let first = index(term.point_from_grid_ref(&word.start(), PointSpace::Viewport).ok()??);
    let start = first + text[..range.start].chars().count() as u64;
    let end = first + text[..range.end].chars().count() as u64 - 1;
    (start..=end).contains(&index(cell)).then(|| Link {
        url: text[range].to_owned(),
        start: point(start),
        end: point(end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_vt::terminal::Options;

    fn link(term: &Terminal, x: u16, y: u32) -> Option<String> {
        link_at(term, PointCoordinate { x, y }).map(|link| link.url)
    }

    #[test]
    fn links() {
        let mut term = Terminal::new(Options {
            cols: 20,
            rows: 5,
            ..Options::default()
        })
        .unwrap();
        term.vt_write(b"a \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ b\r\n");
        term.vt_write(b"see https://xconn.io/docs/intro.");
        assert_eq!(link(&term, 3, 0).as_deref(), Some("https://example.com"));
        assert_eq!(link(&term, 7, 0), None);
        // The URL wraps onto the next row.
        assert_eq!(link(&term, 2, 2).as_deref(), Some("https://xconn.io/docs/intro"));
        assert_eq!(link(&term, 1, 1), None);
    }
}
