//! A tab's terminal: the session's output drawn as alacritty's grid, and the user's keys sent
//! to it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
use gpui::{
    App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla,
    KeyDownEvent, MouseButton, PaintQuad, Pixels, ScrollWheelEvent, ShapedLine, StrikethroughStyle, TextAlign, TextRun,
    UnderlineStyle, Window, actions, canvas, div, fill, outline, point, prelude::*, px, size,
};
use tokio::sync::mpsc;

use crate::daemon::Machine;
use crate::session::{self, Command, Event};
use crate::{keys, theme};

actions!(terminal, [Paste]);

const FONT_SIZE: Pixels = px(14.);
const LINE_HEIGHT: f32 = 1.3;
const SCROLLBACK: usize = 10_000;

/// The session ended: the tab is to close.
pub struct Closed;

pub struct TerminalView {
    pub machine: Machine,
    term: Term<Listener>,
    parser: Processor,
    /// What the terminal asked for while taking in output.
    requests: Rc<RefCell<Vec<TermEvent>>>,
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
    pub fn new(machine: Machine, dir: PathBuf, cx: &mut Context<Self>) -> Self {
        let (cols, rows) = (80, 24);
        let requests = Rc::default();
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        let term = Term::new(config, &GridSize { cols, rows }, Listener(Rc::clone(&requests)));

        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, mut events) = mpsc::unbounded_channel();
        let session = crate::runtime().spawn(session::run(
            dir,
            machine.realm.clone(),
            cols as u16,
            rows as u16,
            rx,
            tx,
        ));
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

        Self {
            machine,
            term,
            parser: Processor::new(),
            requests,
            commands,
            session: Some(session),
            focus: cx.focus_handle(),
            title: None,
            cell: None,
            scroll: px(0.),
        }
    }

    /// Ends the session, and the shell with it, returning its task to wait for.
    pub fn terminate(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let _ = self.commands.send(Command::Terminate);
        self.session.take()
    }

    fn handle(&mut self, events: Vec<Event>, cx: &mut Context<Self>) {
        for event in events {
            match event {
                Event::Output(data) => self.parser.advance(&mut self.term, &data),
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
        self.parser.advance(&mut self.term, text.as_bytes());
    }

    /// Answers what the terminal's programs asked of it.
    fn answer_requests(&mut self, cx: &mut Context<Self>) {
        let requests = std::mem::take(&mut *self.requests.borrow_mut());
        for request in requests {
            match request {
                TermEvent::PtyWrite(text) => self.send(text.into_bytes()),
                TermEvent::Title(title) => self.title = Some(title),
                TermEvent::ResetTitle => self.title = None,
                TermEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                TermEvent::ColorRequest(index, format) => {
                    let color = match index {
                        256 => Color::Named(NamedColor::Foreground),
                        257 => Color::Named(NamedColor::Background),
                        258 => Color::Named(NamedColor::Cursor),
                        i => Color::Indexed(i.min(255) as u8),
                    };
                    let rgb = theme::to_rgb(theme::resolve(color, self.term.colors()));
                    self.send(format(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    if let Some(cell) = self.cell {
                        let size = WindowSize {
                            num_lines: self.term.screen_lines() as u16,
                            num_cols: self.term.columns() as u16,
                            cell_width: f32::from(cell.width) as u16,
                            cell_height: f32::from(cell.height) as u16,
                        };
                        self.send(format(size).into_bytes());
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
        if let Some(bytes) = keys::to_esc(&event.keystroke, *self.term.mode()) {
            self.term.scroll_display(Scroll::Bottom);
            self.send(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        // Pasted lines end as typed ones do.
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let data = if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text
        };
        self.term.scroll_display(Scroll::Bottom);
        self.send(data.into_bytes());
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
        let mode = *self.term.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs scroll with the arrow keys.
            let key = if lines > 0 { b'A' } else { b'B' };
            let prefix: &[u8] = if mode.contains(TermMode::APP_CURSOR) {
                b"\x1bO"
            } else {
                b"\x1b["
            };
            let mut data = Vec::new();
            for _ in 0..lines.unsigned_abs() {
                data.extend_from_slice(prefix);
                data.push(key);
            }
            self.send(data);
        } else {
            self.term.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }

    /// Sizes the terminal to fit bounds, with cells of cell.
    fn fit(&mut self, bounds: Bounds<Pixels>, cell: gpui::Size<Pixels>) {
        self.cell = Some(cell);
        let cols = ((bounds.size.width / cell.width) as usize).max(2);
        let rows = ((bounds.size.height / cell.height) as usize).max(1);
        if cols != self.term.columns() || rows != self.term.screen_lines() {
            self.term.resize(GridSize { cols, rows });
            let _ = self.commands.send(Command::Resize {
                cols: cols as u16,
                rows: rows as u16,
            });
        }
    }

    /// Shapes what's on screen, for paint.
    fn layout(&self, bounds: Bounds<Pixels>, cell: gpui::Size<Pixels>, focused: bool, window: &mut Window) -> Layout {
        let content = self.term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset as i32;
        let cursor = content.cursor;
        let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && cursor.shape != CursorShape::Hidden;
        let block_cursor = show_cursor && focused && cursor.shape == CursorShape::Block;
        let cursor_color = theme::resolve(Color::Named(NamedColor::Cursor), colors);
        let default_bg = theme::resolve(Color::Named(NamedColor::Background), colors);
        let at = |row: i32, col: usize| bounds.origin + point(cell.width * col as f32, cell.height * row as f32);

        let mut layout = Layout {
            backgrounds: Vec::new(),
            lines: Vec::new(),
            cursor: None,
        };
        let mut segment: Option<Segment> = None;
        for indexed in content.display_iter {
            let flags = indexed.cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let row = indexed.point.line.0 + offset;
            let col = indexed.point.column.0;
            let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

            let fg = if flags.contains(Flags::BOLD) {
                theme::brighten(indexed.cell.fg)
            } else {
                indexed.cell.fg
            };
            let mut fg = theme::resolve(fg, colors);
            let mut bg = theme::resolve(indexed.cell.bg, colors);
            if flags.contains(Flags::DIM) {
                fg = fg.opacity(0.66);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if block_cursor && indexed.point == cursor.point {
                (fg, bg) = (default_bg, cursor_color);
            }

            if bg != default_bg {
                let bounds = Bounds::new(at(row, col), size(cell.width * width as f32, cell.height));
                match layout.backgrounds.last_mut() {
                    Some((last, color)) if *color == bg && last.top_right() == bounds.origin => {
                        last.size.width += bounds.size.width;
                    }
                    _ => layout.backgrounds.push((bounds, bg)),
                }
            }

            let style = Style::new(fg, flags);
            let continues = segment
                .as_ref()
                .is_some_and(|s| s.row == row && s.col + s.cells == col && s.style == style && width == 1 && !s.wide);
            if !continues {
                if let Some(done) = segment.take() {
                    layout
                        .lines
                        .extend(done.shape(cell, window).map(|line| (at(done.row, done.col), line)));
                }
                segment = Some(Segment {
                    row,
                    col,
                    cells: 0,
                    wide: width == 2,
                    style,
                    text: String::new(),
                });
            }
            let s = segment.as_mut().expect("a segment was just started");
            s.text.push(indexed.cell.c);
            s.text.extend(indexed.cell.zerowidth().into_iter().flatten());
            s.cells += width;
        }
        if let Some(done) = segment {
            layout
                .lines
                .extend(done.shape(cell, window).map(|line| (at(done.row, done.col), line)));
        }

        let cursor_row = cursor.point.line.0 + offset;
        if show_cursor && !block_cursor && (0..self.term.screen_lines() as i32).contains(&cursor_row) {
            let origin = at(cursor_row, cursor.point.column.0);
            layout.cursor = Some(match cursor.shape {
                CursorShape::Beam => fill(Bounds::new(origin, size(px(2.), cell.height)), cursor_color),
                CursorShape::Underline => fill(
                    Bounds::new(origin + point(px(0.), cell.height - px(2.)), size(cell.width, px(2.))),
                    cursor_color,
                ),
                _ => outline(Bounds::new(origin, cell), cursor_color, gpui::BorderStyle::Solid),
            });
        }
        layout
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
                        view.update(cx, |view, _| view.fit(bounds, cell));
                        view.read(cx).layout(bounds, cell, focused, window)
                    },
                    move |_bounds, layout, window, cx| layout.paint(window, cx),
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
struct Style {
    fg: Hsla,
    flags: Flags,
}

impl Style {
    fn new(fg: Hsla, flags: Flags) -> Self {
        let drawn = Flags::BOLD | Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT;
        Self {
            fg,
            flags: flags & drawn,
        }
    }
}

/// Cells next to each other on a row, drawn alike.
struct Segment {
    row: i32,
    col: usize,
    cells: usize,
    /// A wide character, which has a segment of its own.
    wide: bool,
    style: Style,
    text: String,
}

impl Segment {
    /// The segment shaped, unless there's nothing to draw.
    fn shape(&self, cell: gpui::Size<Pixels>, window: &mut Window) -> Option<ShapedLine> {
        let flags = self.style.flags;
        if self.text.trim().is_empty() && !flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
            return None;
        }
        let mut font = terminal_font();
        if flags.contains(Flags::BOLD) {
            font.weight = FontWeight::BOLD;
        }
        if flags.contains(Flags::ITALIC) {
            font.style = FontStyle::Italic;
        }
        let run = TextRun {
            len: self.text.len(),
            font,
            color: self.style.fg,
            background_color: None,
            underline: flags.intersects(Flags::ALL_UNDERLINES).then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(self.style.fg),
                wavy: flags.contains(Flags::UNDERCURL),
            }),
            strikethrough: flags.contains(Flags::STRIKEOUT).then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(self.style.fg),
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

/// The terminal's size, in cells.
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// Collects what the terminal asks for, to be answered once the output is taken in.
struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.borrow_mut().push(event);
    }
}
