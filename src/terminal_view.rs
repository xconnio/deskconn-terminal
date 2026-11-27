//! A tab's terminal: the session's output taken in by libghostty-vt and drawn from its
//! render state, and the user's keys sent to it.

use std::path::PathBuf;

use ghostty_vt::Terminal;
use ghostty_vt::render::{CellIterator, CursorVisualStyle, RenderState, RowIterator};
use ghostty_vt::screen::{CellWide, Screen};
use ghostty_vt::style::{StyleColor, Underline};
use ghostty_vt::terminal::{Event as TermEvent, Mode, Options, ScrollViewport};
use gpui::{
    App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla,
    KeyDownEvent, MouseButton, PaintQuad, Pixels, ScrollWheelEvent, ShapedLine, StrikethroughStyle, TextAlign, TextRun,
    UnderlineStyle, Window, actions, canvas, div, fill, outline, point, prelude::*, px, size,
};
use tokio::sync::mpsc;

use crate::daemon::Machine;
use crate::keys::KeyEncoder;
use crate::session::{self, Command, Event};
use crate::theme;

actions!(terminal, [Paste]);

const FONT_SIZE: Pixels = px(14.);
const LINE_HEIGHT: f32 = 1.3;

/// The session ended: the tab is to close.
pub struct Closed;

pub struct TerminalView {
    pub machine: Machine,
    term: Terminal,
    render: RenderState,
    rows: RowIterator,
    cells: CellIterator,
    keys: KeyEncoder,
    commands: mpsc::UnboundedSender<Command>,
    /// The session's task, until it's ended.
    session: Option<tokio::task::JoinHandle<()>>,
    focus: FocusHandle,
    /// The title the shell set, if any.
    pub title: Option<String>,
    /// The size of a cell, once laid out.
    cell: Option<gpui::Size<Pixels>>,
    /// Scrolling not yet a whole line.
    scroll: Pixels,
}

impl EventEmitter<Closed> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    /// Starts a new session on machine, through the daemon of dir.
    pub fn new(machine: Machine, dir: PathBuf, cx: &mut Context<Self>) -> ghostty_vt::error::Result<Self> {
        let (cols, rows) = (80, 24);
        let mut term = Terminal::new(Options {
            cols,
            rows,
            ..Options::default()
        })?;
        let (fg, bg) = theme::terminal_colors();
        let palette = theme::palette(term.default_color_palette()?);
        term.set_default_fg_color(Some(fg))?
            .set_default_bg_color(Some(bg))?
            .set_default_color_palette(Some(palette))?;
        term.set_size_reports(true);

        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, mut events) = mpsc::unbounded_channel();
        let session = crate::runtime().spawn(session::run(dir, machine.realm.clone(), cols, rows, rx, tx));
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                // Take in what's arrived since in one go, and draw once.
                let mut batch = vec![event];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |view, cx| view.handle(batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        Ok(Self {
            machine,
            term,
            render: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            keys: KeyEncoder::new()?,
            commands,
            session: Some(session),
            focus: cx.focus_handle(),
            title: None,
            cell: None,
            scroll: px(0.),
        })
    }

    /// Ends the session, and the shell with it, returning its task to wait for.
    pub fn terminate(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let _ = self.commands.send(Command::Terminate);
        self.session.take()
    }

    fn handle(&mut self, events: Vec<Event>, cx: &mut Context<Self>) {
        for event in events {
            match event {
                Event::Output(data) => self.term.vt_write(&data),
                Event::Reconnecting => self.notice("Connection lost; reconnecting…"),
                Event::Reconnected => {}
                Event::Exited(_) => {
                    cx.emit(Closed);
                    return;
                }
                Event::Displaced => self.notice("This session was opened elsewhere."),
                Event::Failed(message) => self.notice(&message),
            }
        }
        self.answer_requests(cx);
        cx.notify();
    }

    /// Shows a message from the app on a line of its own.
    fn notice(&mut self, message: &str) {
        let text = format!("\x1b[0m\r\n\x1b[2m[deskconn] {message}\x1b[0m\r\n");
        self.term.vt_write(text.as_bytes());
    }

    /// Answers what the terminal's programs asked of it, and takes in what they told it.
    fn answer_requests(&mut self, cx: &mut Context<Self>) {
        let events: Vec<_> = self.term.events().collect();
        for event in events {
            match event {
                TermEvent::PtyWrite(data) => self.send(data),
                TermEvent::TitleChanged(title) => self.title = Some(title).filter(|title| !title.is_empty()),
                TermEvent::ClipboardWrite(write) => {
                    let text = write
                        .contents
                        .into_iter()
                        .find(|content| content.mime.starts_with("text/plain"));
                    if let Some(text) = text {
                        cx.write_to_clipboard(ClipboardItem::new_string(String::from_utf8_lossy(&text.data).into()));
                    }
                }
                _ => {}
            }
        }
    }

    fn send(&self, data: Vec<u8>) {
        let _ = self.commands.send(Command::Input(data));
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let bytes = self.keys.encode(&event.keystroke, &self.term);
        if !bytes.is_empty() {
            self.term.scroll_viewport(ScrollViewport::Bottom);
            self.send(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bracketed = self.term.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.into_bytes();
        // Room for the brackets around it.
        let mut encoded = vec![0; data.len() + 16];
        let Ok(n) = ghostty_vt::paste::encode(&mut data, bracketed, &mut encoded) else {
            return;
        };
        encoded.truncate(n);
        self.term.scroll_viewport(ScrollViewport::Bottom);
        self.send(encoded);
        cx.notify();
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(cell) = self.cell else { return };
        self.scroll += event.delta.pixel_delta(cell.height).y;
        let lines = (self.scroll / cell.height) as i32;
        if lines == 0 {
            return;
        }
        self.scroll -= cell.height * lines as f32;
        let alternate = self
            .term
            .active_screen()
            .is_ok_and(|screen| screen == Screen::Alternate);
        if alternate && self.term.mode(Mode::ALT_SCROLL).unwrap_or(false) {
            // Full-screen programs scroll with the arrow keys.
            let prefix: &[u8] = if self.term.mode(Mode::DECCKM).unwrap_or(false) {
                b"\x1bO"
            } else {
                b"\x1b["
            };
            let key = if lines > 0 { b'A' } else { b'B' };
            let data = (0..lines.unsigned_abs())
                .flat_map(|_| prefix.iter().copied().chain([key]))
                .collect();
            self.send(data);
        } else {
            self.term.scroll_viewport(ScrollViewport::Delta(-lines as isize));
            cx.notify();
        }
    }

    /// Sizes the terminal to fit bounds, with cells of cell.
    fn fit(&mut self, bounds: Bounds<Pixels>, cell: gpui::Size<Pixels>) {
        self.cell = Some(cell);
        let cols = ((bounds.size.width / cell.width) as u16).max(2);
        let rows = ((bounds.size.height / cell.height) as u16).max(1);
        if (Ok(cols), Ok(rows)) != (self.term.cols(), self.term.rows()) {
            let (w, h) = (f32::from(cell.width) as u32, f32::from(cell.height) as u32);
            if self.term.resize(cols, rows, w, h).is_ok() {
                let _ = self.commands.send(Command::Resize { cols, rows });
            }
        }
    }

    /// Shapes what's on screen, for paint.
    fn layout(
        &mut self,
        bounds: Bounds<Pixels>,
        cell: gpui::Size<Pixels>,
        focused: bool,
        window: &mut Window,
    ) -> ghostty_vt::error::Result<Layout> {
        let snapshot = self.render.update(&self.term)?;
        let colors = snapshot.colors()?;
        let cursor = snapshot.cursor()?;
        let default_fg = theme::to_hsla(colors.foreground);
        let default_bg = theme::to_hsla(colors.background);
        let cursor_color = colors.cursor.map_or(default_fg, theme::to_hsla);
        let cursor_at = cursor.viewport.filter(|_| cursor.visible);
        let block_cursor = focused && cursor.visual_style == CursorVisualStyle::Block;
        let at = |row: u16, col: u16| bounds.origin + point(cell.width * f32::from(col), cell.height * f32::from(row));

        let mut layout = Layout {
            backgrounds: Vec::new(),
            lines: Vec::new(),
            cursor: None,
        };
        let mut text = String::new();
        let mut rows = self.rows.update(&snapshot)?;
        let mut y = 0;
        while let Some(row) = rows.next() {
            let mut segment: Option<Segment> = None;
            let mut cells = self.cells.update(row)?;
            let mut x = 0;
            while let Some(c) = cells.next() {
                let col = x;
                x += 1;
                let width = match c.raw_cell()?.wide()? {
                    CellWide::SpacerTail | CellWide::SpacerHead => continue,
                    CellWide::Wide => 2,
                    _ => 1,
                };
                let style = c.style()?;
                // Bold text of a normal ANSI color is drawn in its bright variant.
                let fg = match style.fg_color {
                    StyleColor::Palette(i) if style.bold && i.0 < 8 => Some(colors.palette[i.0 as usize + 8]),
                    _ => c.fg_color()?,
                };
                let mut fg = fg.map_or(default_fg, theme::to_hsla);
                let mut bg = c.bg_color()?.map_or(default_bg, theme::to_hsla);
                if style.faint {
                    fg = fg.opacity(0.66);
                }
                if style.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if style.invisible {
                    fg = bg;
                }
                if block_cursor && cursor_at.is_some_and(|at| (at.x, at.y) == (col, y)) {
                    (fg, bg) = (default_bg, cursor_color);
                }

                if bg != default_bg {
                    let bounds = Bounds::new(at(y, col), size(cell.width * width as f32, cell.height));
                    match layout.backgrounds.last_mut() {
                        Some((last, color)) if *color == bg && last.top_right() == bounds.origin => {
                            last.size.width += bounds.size.width;
                        }
                        _ => layout.backgrounds.push((bounds, bg)),
                    }
                }

                text.clear();
                c.graphemes_utf8(&mut text)?;
                if text.is_empty() {
                    text.push(' ');
                }
                let look = Look {
                    fg,
                    bold: style.bold,
                    italic: style.italic,
                    underline: style.underline,
                    strike: style.strikethrough,
                };
                let continues = segment
                    .as_ref()
                    .is_some_and(|s| s.col + s.cells == col && s.look == look && width == 1 && !s.wide);
                if !continues {
                    if let Some(done) = segment.take() {
                        layout
                            .lines
                            .extend(done.shape(cell, window).map(|line| (at(y, done.col), line)));
                    }
                    segment = Some(Segment {
                        col,
                        cells: 0,
                        wide: width == 2,
                        look,
                        text: String::new(),
                    });
                }
                let s = segment.as_mut().expect("a segment was just started");
                s.text.push_str(&text);
                s.cells += width;
            }
            if let Some(done) = segment {
                layout
                    .lines
                    .extend(done.shape(cell, window).map(|line| (at(y, done.col), line)));
            }
            y += 1;
        }

        if let Some(cursor_at) = cursor_at.filter(|_| !block_cursor) {
            let origin = at(cursor_at.y, cursor_at.x);
            layout.cursor = Some(match cursor.visual_style {
                CursorVisualStyle::Bar => fill(Bounds::new(origin, size(px(2.), cell.height)), cursor_color),
                CursorVisualStyle::Underline => fill(
                    Bounds::new(origin + point(px(0.), cell.height - px(2.)), size(cell.width, px(2.))),
                    cursor_color,
                ),
                _ => outline(Bounds::new(origin, cell), cursor_color, gpui::BorderStyle::Solid),
            });
        }
        snapshot.clean()?;
        Ok(layout)
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::paste))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| view.focus.focus(window, cx)),
            )
            .size_full()
            .px_2()
            .pt_1()
            .bg(theme::background())
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let cell = cell_size(window);
                        view.update(cx, |view, _| {
                            view.fit(bounds, cell);
                            view.layout(bounds, cell, focused, window).ok()
                        })
                    },
                    move |_bounds, layout, window, cx| {
                        if let Some(layout) = layout {
                            layout.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            )
    }
}

/// The size of a cell in the terminal's font.
fn cell_size(window: &Window) -> gpui::Size<Pixels> {
    let text = window.text_system();
    let font_id = text.resolve_font(&terminal_font());
    let width = text
        .advance(font_id, FONT_SIZE, 'm')
        .map_or(FONT_SIZE * 0.6, |advance| advance.width);
    size(width, (FONT_SIZE * LINE_HEIGHT).round())
}

fn terminal_font() -> Font {
    let family = if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    };
    gpui::font(family)
}

/// What's on screen, ready to paint.
struct Layout {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(gpui::Point<Pixels>, ShapedLine)>,
    cursor: Option<PaintQuad>,
}

impl Layout {
    fn paint(self, window: &mut Window, cx: &mut App) {
        let line_height = (FONT_SIZE * LINE_HEIGHT).round();
        for (bounds, color) in self.backgrounds {
            window.paint_quad(fill(bounds, color));
        }
        for (origin, line) in self.lines {
            let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
        }
        if let Some(cursor) = self.cursor {
            window.paint_quad(cursor);
        }
    }
}

/// How a run of cells is drawn.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: Underline,
    strike: bool,
}

/// Cells next to each other on a row, drawn alike.
struct Segment {
    col: u16,
    cells: u16,
    /// A wide character, which has a segment of its own.
    wide: bool,
    look: Look,
    text: String,
}

impl Segment {
    /// The segment shaped, unless there's nothing to draw.
    fn shape(&self, cell: gpui::Size<Pixels>, window: &mut Window) -> Option<ShapedLine> {
        let look = self.look;
        let decorated = look.underline != Underline::None || look.strike;
        if self.text.trim().is_empty() && !decorated {
            return None;
        }
        let mut font = terminal_font();
        if look.bold {
            font.weight = FontWeight::BOLD;
        }
        if look.italic {
            font.style = FontStyle::Italic;
        }
        let run = TextRun {
            len: self.text.len(),
            font,
            color: look.fg,
            background_color: None,
            underline: (look.underline != Underline::None).then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(look.fg),
                wavy: look.underline == Underline::Curly,
            }),
            strikethrough: look.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(look.fg),
            }),
        };
        // Glyphs from fallback fonts are kept to the grid, but for wide characters.
        let force_width = (!self.wide).then_some(cell.width);
        Some(
            window
                .text_system()
                .shape_line(self.text.clone().into(), FONT_SIZE, &[run], force_width),
        )
    }
}
