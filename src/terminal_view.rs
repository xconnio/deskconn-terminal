//! A tab's terminal: the session's output taken in by libghostty-vt and drawn from its
//! render state, and the user's keys sent to it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ghostty_vt::Terminal;
use ghostty_vt::render::{
    CellIteration, CellIterator, Colors, CursorVisualStyle, Dirty, RenderState, RowId, RowIterator,
};
use ghostty_vt::screen::{CellWide, Screen};
use ghostty_vt::style::{StyleColor, Underline};
use ghostty_vt::terminal::{Event as TermEvent, Mode, Options, ScrollViewport};
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight,
    Global, Hitbox, HitboxBehavior, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollWheelEvent, ShapedLine, Size, StrikethroughStyle, Task, TextAlign, TextRun,
    UnderlineStyle, Window, actions, canvas, div, fill, hsla, outline, point, prelude::*, px, size,
};
use tokio::sync::mpsc;

use crate::daemon::Machine;
use crate::keys::KeyEncoder;
use crate::session::{self, Command, Event};
use crate::theme;

actions!(terminal, [Paste, IncreaseFontSize, DecreaseFontSize, ResetFontSize]);

const DEFAULT_FONT_SIZE: Pixels = px(14.);
/// The smallest and largest font sizes the user can pick.
const FONT_SIZES: (f32, f32) = (8., 40.);
const LINE_HEIGHT: f32 = 1.3;
/// How long the terminal's size has to stay the same before the shell is told of it.
const RESIZE_DELAY: Duration = Duration::from_millis(100);
/// How long a blinking cursor stays shown, and then hidden.
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// How long the cursor blinks after the last key, as GNOME has it, before it stays shown.
const BLINK_TIMEOUT: Duration = Duration::from_secs(10);
/// The scrollbar's width, within the terminal's right padding, and its thumb's least height.
const SCROLLBAR_WIDTH: Pixels = px(6.);
const SCROLLBAR_MIN_THUMB: Pixels = px(24.);

/// The terminals' font size, as the user set it.
struct FontSize(Pixels);

impl Global for FontSize {}

fn font_size(cx: &App) -> Pixels {
    cx.try_global::<FontSize>().map_or(DEFAULT_FONT_SIZE, |size| size.0)
}

/// Sets every terminal's font size.
fn set_font_size(size: Pixels, cx: &mut App) {
    let (min, max) = FONT_SIZES;
    cx.set_global(FontSize(px(f32::from(size).clamp(min, max))));
    cx.refresh_windows();
}

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
    cell: Option<Size<Pixels>>,
    /// The font size the rows were shaped in.
    font_size: Pixels,
    /// The rows last laid out, to draw again while they're unchanged.
    shaped: HashMap<RowId, Rc<ShapedRow>>,
    /// Telling the shell of a new size, once it has settled.
    resize: Option<Task<()>>,
    /// Scrolling not yet a whole line.
    scroll: Pixels,
    /// Whether the cursor blinks, as programs can set, and whether it's shown just now.
    cursor_blinks: bool,
    blink_on: bool,
    blink: Task<()>,
    /// Where the scrollbar's thumb was grabbed, from its top, while it's being dragged.
    scrollbar_drag: Option<(Pixels, Scrollbar)>,
}

impl EventEmitter<Closed> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    /// Starts a new session on machine, through the daemon of dir.
    pub fn new(
        machine: Machine,
        dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ghostty_vt::error::Result<Self> {
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
        term.set_default_cursor_blink(Some(true))?;

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

        let focus = cx.focus_handle();
        cx.on_focus(&focus, window, Self::restart_blink).detach();

        Ok(Self {
            machine,
            term,
            render: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            keys: KeyEncoder::new()?,
            commands,
            session: Some(session),
            focus,
            title: None,
            cell: None,
            font_size: DEFAULT_FONT_SIZE,
            shaped: HashMap::new(),
            resize: None,
            scroll: px(0.),
            cursor_blinks: false,
            blink_on: true,
            blink: Task::ready(()),
            scrollbar_drag: None,
        })
    }

    /// Shows the cursor, to blink for a while from now on, while the terminal has focus.
    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blink_on = true;
        cx.notify();
        let started = Instant::now();
        self.blink = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let done = started.elapsed() >= BLINK_TIMEOUT;
                let blinking = this.update_in(cx, |view, window, cx| {
                    let blinking = !done && view.focus.is_focused(window);
                    view.blink_on = !view.blink_on || !blinking;
                    if view.cursor_blinks {
                        cx.notify();
                    }
                    blinking
                });
                if !matches!(blinking, Ok(true)) {
                    break;
                }
            }
        });
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let bytes = self.keys.encode(&event.keystroke, &self.term);
        if !bytes.is_empty() {
            self.restart_blink(window, cx);
            self.term.scroll_viewport(ScrollViewport::Bottom);
            self.send(bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
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
        self.restart_blink(window, cx);
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

    /// Starts dragging the scrollbar's thumb from y: where it's grabbed, or from its middle,
    /// moved there, when the track is clicked.
    fn grab_scrollbar(&mut self, bar: Scrollbar, y: Pixels, cx: &mut Context<Self>) {
        let grab = if bar.thumb.top() <= y && y <= bar.thumb.bottom() {
            y - bar.thumb.top()
        } else {
            bar.thumb.size.height / 2.
        };
        self.scrollbar_drag = Some((grab, bar));
        self.drag_scrollbar(y, cx);
    }

    /// Scrolls to where the thumb, dragged to y, is on the track.
    fn drag_scrollbar(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some((grab, bar)) = self.scrollbar_drag else {
            return;
        };
        let room = bar.track.size.height - bar.thumb.size.height;
        let fraction = if room > px(0.) {
            ((y - grab - bar.track.top()) / room).clamp(0., 1.)
        } else {
            1.
        };
        let row = (fraction * bar.rows as f32).round() as usize;
        self.term.scroll_viewport(ScrollViewport::Row(row));
        cx.notify();
    }

    /// Sizes the terminal to fit bounds, with cells of cell. The shell hears of a new size once it
    /// has settled, so that dragging the window's edge doesn't make it redraw at every step.
    fn fit(&mut self, bounds: Bounds<Pixels>, font_size: Pixels, cell: Size<Pixels>, cx: &mut Context<Self>) {
        if (self.cell, self.font_size) != (Some(cell), font_size) {
            (self.cell, self.font_size) = (Some(cell), font_size);
            self.shaped.clear();
        }
        let cols = ((bounds.size.width / cell.width) as u16).max(2);
        let rows = ((bounds.size.height / cell.height) as u16).max(1);
        if (Ok(cols), Ok(rows)) == (self.term.cols(), self.term.rows()) {
            return;
        }
        let (width, height) = (f32::from(cell.width) as u32, f32::from(cell.height) as u32);
        if self.term.resize(cols, rows, width, height).is_ok() {
            self.resize = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(RESIZE_DELAY).await;
                let _ = this.update(cx, |view, _| view.commands.send(Command::Resize { cols, rows }));
            }));
        }
    }

    /// Lays out what's on screen, for paint. Rows that haven't changed keep their shaped text.
    fn layout(
        &mut self,
        bounds: Bounds<Pixels>,
        font_size: Pixels,
        cell: Size<Pixels>,
        focused: bool,
        window: &mut Window,
    ) -> ghostty_vt::error::Result<Layout> {
        let scrollbar = self.scrollbar(bounds);
        let snapshot = self.render.update(&self.term)?;
        if snapshot.dirty()? == Dirty::Full {
            self.shaped.clear();
        }
        let colors = snapshot.colors()?;
        let cursor = snapshot.cursor()?;
        self.cursor_blinks = cursor.blinking;
        let at = |row: u16, col: u16| bounds.origin + point(cell.width * f32::from(col), cell.height * f32::from(row));

        let mut layout = Layout {
            line_height: cell.height,
            rows: Vec::new(),
            cursor: None,
            scrollbar,
            scrollbar_hitbox: None,
        };
        // A blinking cursor is hidden every other interval, while the terminal has focus.
        let shown = cursor.visible && (!cursor.blinking || !focused || self.blink_on);
        let mut shaped = HashMap::with_capacity(self.shaped.len());
        let mut rows = self.rows.update(&snapshot)?;
        let mut y = 0;
        while let Some(row) = rows.next() {
            let id = row.id()?;
            let line = match self.shaped.remove(&id).filter(|_| !row.dirty().unwrap_or(true)) {
                Some(line) => line,
                None => Rc::new(shape_row(
                    &mut self.cells.update(row)?,
                    &colors,
                    font_size,
                    cell,
                    window,
                )?),
            };
            layout.rows.push((at(y, 0), line.clone()));
            shaped.insert(id, line);

            if let Some(position) = cursor.viewport.filter(|position| shown && position.y == y) {
                let x = if position.at_wide_tail {
                    position.x.saturating_sub(1)
                } else {
                    position.x
                };
                let mut cells = self.cells.update(row)?;
                cells.select(x)?;
                let wide = cells.raw_cell()?.wide()? == CellWide::Wide;
                let origin = at(y, x);
                let width = if wide { cell.width * 2. } else { cell.width };
                let color = colors.cursor.unwrap_or(colors.foreground);
                layout.cursor = Some(match cursor.visual_style {
                    CursorVisualStyle::Block if focused => {
                        // The character under the cursor, in the background's color.
                        let style = cells.style()?;
                        let mut text = String::new();
                        cells.graphemes_utf8(&mut text)?;
                        let look = Look {
                            fg: theme::to_hsla(colors.background),
                            bold: style.bold,
                            italic: style.italic,
                            underline: Underline::None,
                            strike: false,
                        };
                        let glyph = Segment {
                            col: x,
                            cells: 1 + u16::from(wide),
                            wide,
                            look,
                            text,
                        }
                        .shape(font_size, cell, window);
                        Cursor {
                            quad: fill(Bounds::new(origin, size(width, cell.height)), theme::to_hsla(color)),
                            glyph: glyph.map(|glyph| (origin, glyph)),
                        }
                    }
                    CursorVisualStyle::Bar => Cursor {
                        quad: fill(Bounds::new(origin, size(px(2.), cell.height)), theme::to_hsla(color)),
                        glyph: None,
                    },
                    CursorVisualStyle::Underline => Cursor {
                        quad: fill(
                            Bounds::new(origin + point(px(0.), cell.height - px(2.)), size(width, px(2.))),
                            theme::to_hsla(color),
                        ),
                        glyph: None,
                    },
                    _ => Cursor {
                        quad: outline(
                            Bounds::new(origin, size(width, cell.height)),
                            theme::to_hsla(color),
                            gpui::BorderStyle::Solid,
                        ),
                        glyph: None,
                    },
                });
            }
            y += 1;
        }
        self.shaped = shaped;
        snapshot.clean()?;
        Ok(layout)
    }

    /// The scrollbar, in the padding right of bounds, when there's scrollback to scroll through.
    fn scrollbar(&self, bounds: Bounds<Pixels>) -> Option<Scrollbar> {
        let bar = self.term.scrollbar().ok()?;
        let rows = bar.total.saturating_sub(bar.len);
        if rows == 0 {
            return None;
        }
        // Kept off the bottom edge, where the window's corner is rounded.
        let track = Bounds::new(
            point(bounds.right() + px(1.), bounds.top()),
            size(SCROLLBAR_WIDTH, bounds.size.height - px(6.)),
        );
        let track_height = f32::from(track.size.height);
        let height = (track_height * bar.len as f32 / bar.total as f32)
            .max(f32::from(SCROLLBAR_MIN_THUMB))
            .min(track_height);
        let top = track.top() + px((track_height - height) * bar.offset as f32 / rows as f32);
        Some(Scrollbar {
            track,
            thumb: Bounds::new(point(track.left(), top), size(SCROLLBAR_WIDTH, px(height))),
            rows,
        })
    }
}

/// Shapes a row's text, and lays out the backgrounds that differ from the terminal's.
fn shape_row(
    cells: &mut CellIteration<'_>,
    colors: &Colors,
    font_size: Pixels,
    cell: Size<Pixels>,
    window: &mut Window,
) -> ghostty_vt::error::Result<ShapedRow> {
    let default_fg = theme::to_hsla(colors.foreground);
    let default_bg = theme::to_hsla(colors.background);
    let at = |col: u16| point(cell.width * f32::from(col), px(0.));
    let mut row = ShapedRow {
        backgrounds: Vec::new(),
        lines: Vec::new(),
    };
    let mut segment: Option<Segment> = None;
    let mut text = String::new();
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

        if bg != default_bg {
            let bounds = Bounds::new(at(col), size(cell.width * f32::from(width), cell.height));
            match row.backgrounds.last_mut() {
                Some((last, color)) if *color == bg && last.top_right() == bounds.origin => {
                    last.size.width += bounds.size.width;
                }
                _ => row.backgrounds.push((bounds, bg)),
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
                row.lines
                    .extend(done.shape(font_size, cell, window).map(|line| (at(done.col), line)));
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
        row.lines
            .extend(done.shape(font_size, cell, window).map(|line| (at(done.col), line)));
    }
    Ok(row)
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let painted = view.clone();
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::paste))
            .on_action(|_: &IncreaseFontSize, _, cx| set_font_size(font_size(cx) + px(1.), cx))
            .on_action(|_: &DecreaseFontSize, _, cx| set_font_size(font_size(cx) - px(1.), cx))
            .on_action(|_: &ResetFontSize, _, cx| set_font_size(DEFAULT_FONT_SIZE, cx))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| view.focus.focus(window, cx)),
            )
            .size_full()
            .px_2()
            .pt_1()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let font_size = font_size(cx);
                        let cell = cell_size(font_size, window);
                        let mut layout = view.update(cx, |view, cx| {
                            view.fit(bounds, font_size, cell, cx);
                            view.layout(bounds, font_size, cell, focused, window).ok()
                        })?;
                        layout.scrollbar_hitbox = layout
                            .scrollbar
                            .map(|bar| window.insert_hitbox(bar.track, HitboxBehavior::BlockMouse));
                        Some(layout)
                    },
                    move |_bounds, layout, window, cx| {
                        if let Some(layout) = layout {
                            layout.paint(&painted, window, cx);
                        }
                    },
                )
                .size_full(),
            )
    }
}

/// The size of a cell in the terminal's font, at font_size.
fn cell_size(font_size: Pixels, window: &Window) -> Size<Pixels> {
    let text = window.text_system();
    let font_id = text.resolve_font(&terminal_font());
    let width = text
        .advance(font_id, font_size, 'm')
        .map_or(font_size * 0.6, |advance| advance.width);
    size(width, (font_size * LINE_HEIGHT).round())
}

/// JetBrains Mono, which the app brings along, so that it looks the same everywhere.
const FONT_FAMILY: &str = "JetBrains Mono";

/// The fonts of the terminal, to register with GPUI's text system.
pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf")),
    ]
}

fn terminal_font() -> Font {
    gpui::font(FONT_FAMILY)
}

/// What's on screen, ready to paint.
struct Layout {
    line_height: Pixels,
    rows: Vec<(Point<Pixels>, Rc<ShapedRow>)>,
    cursor: Option<Cursor>,
    scrollbar: Option<Scrollbar>,
    scrollbar_hitbox: Option<Hitbox>,
}

/// The scrollbar's track and thumb, and how many rows there are to scroll through.
#[derive(Clone, Copy)]
struct Scrollbar {
    track: Bounds<Pixels>,
    thumb: Bounds<Pixels>,
    rows: u64,
}

/// A row's backgrounds and text, placed from the row's origin.
struct ShapedRow {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
}

/// The cursor, and the character it covers when it's a block.
struct Cursor {
    quad: PaintQuad,
    glyph: Option<(Point<Pixels>, ShapedLine)>,
}

impl Layout {
    fn paint(self, view: &Entity<TerminalView>, window: &mut Window, cx: &mut App) {
        let line_height = self.line_height;
        for (origin, row) in &self.rows {
            for (bounds, color) in &row.backgrounds {
                window.paint_quad(fill(Bounds::new(*origin + bounds.origin, bounds.size), *color));
            }
        }
        for (origin, row) in &self.rows {
            for (at, line) in &row.lines {
                let _ = line.paint(*origin + *at, line_height, TextAlign::Left, None, window, cx);
            }
        }
        if let Some(cursor) = self.cursor {
            window.paint_quad(cursor.quad);
            if let Some((origin, glyph)) = cursor.glyph {
                let _ = glyph.paint(origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
        if let (Some(bar), Some(hitbox)) = (self.scrollbar, self.scrollbar_hitbox) {
            paint_scrollbar(bar, hitbox, view, window, cx);
        }
    }
}

/// Paints the scrollbar, and lets its thumb be dragged, or the track clicked to scroll there.
fn paint_scrollbar(bar: Scrollbar, hitbox: Hitbox, view: &Entity<TerminalView>, window: &mut Window, cx: &mut App) {
    let dragging = view.read(cx).scrollbar_drag.is_some();
    let opacity = if dragging {
        0.5
    } else if hitbox.is_hovered(window) {
        0.35
    } else {
        0.2
    };
    window.paint_quad(fill(bar.thumb, hsla(0., 0., 1., opacity)).corner_radii(SCROLLBAR_WIDTH / 2.));

    let grabbed = view.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase.bubble() && event.button == MouseButton::Left && hitbox.is_hovered(window) {
            grabbed.update(cx, |view, cx| view.grab_scrollbar(bar, event.position.y, cx));
            cx.stop_propagation();
        }
    });
    if dragging {
        // The drag goes on wherever the mouse goes, until it's let go.
        let dragged = view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase.bubble() {
                dragged.update(cx, |view, cx| view.drag_scrollbar(event.position.y, cx));
            }
        });
        let released = view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() && event.button == MouseButton::Left {
                released.update(cx, |view, cx| {
                    view.scrollbar_drag = None;
                    cx.notify();
                });
            }
        });
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
    fn shape(&self, font_size: Pixels, cell: Size<Pixels>, window: &mut Window) -> Option<ShapedLine> {
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
                .shape_line(self.text.clone().into(), font_size, &[run], force_width),
        )
    }
}
