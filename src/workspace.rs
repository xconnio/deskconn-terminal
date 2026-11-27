//! The window: tabs in the title bar, each a terminal on a machine.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, BoxShadow, Context, CursorStyle, Decorations, Entity, FocusHandle, Focusable,
    HitboxBehavior, MouseButton, MouseDownEvent, Pixels, ResizeEdge, SharedString, Tiling, Window, WindowControlArea,
    actions, anchored, canvas, deferred, div, hsla, point, prelude::*, px,
};

use crate::daemon::{self, Machine};
use crate::terminal_view::{Closed, TerminalView};
use crate::theme;

actions!(workspace, [NewTab, CloseTab, NextTab, PreviousTab]);

const TITLEBAR_HEIGHT: Pixels = px(38.);
/// The room around a window that draws its own decorations, for its shadow and its resize
/// handles.
const SHADOW: Pixels = px(10.);
/// How far past its border such a window can be grabbed to resize it, and how much further at
/// its corners.
const RESIZE_INSIDE: Pixels = px(3.);
const RESIZE_CORNER: Pixels = px(8.);
/// The rounding of such a window's corners.
const ROUNDING: Pixels = px(10.);
/// How long closing the window waits for the sessions to end.
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Workspace {
    dir: PathBuf,
    machines: Vec<Machine>,
    tabs: Vec<Entity<TerminalView>>,
    active: usize,
    /// Whether the menu of machines to open a tab on is open.
    menu: bool,
    /// Why the machines couldn't be listed.
    error: Option<String>,
    focus: FocusHandle,
    /// Whether the mouse was pressed on the titlebar, to move the window if it's dragged.
    dragging: bool,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut workspace = Self {
            dir: PathBuf::new(),
            machines: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            menu: false,
            error: None,
            focus: cx.focus_handle(),
            dragging: false,
        };
        match daemon::config_dir().and_then(|dir| Ok((daemon::machines(&dir)?, dir))) {
            Ok((machines, dir)) => {
                workspace.dir = dir;
                workspace.machines = machines;
            }
            Err(err) => workspace.error = Some(format!("{err:#}")),
        }
        // This machine, as the CLI's `desk shell` would open.
        if let Some(machine) = workspace.machines.first().cloned() {
            workspace.open(machine, window, cx);
        }
        cx.on_release(Self::end_sessions).detach();
        workspace
    }

    fn open(&mut self, machine: Machine, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir.clone();
        let view = cx.new(|cx| TerminalView::new(machine, dir, window, cx).expect("libghostty-vt is out of memory"));
        cx.subscribe_in(&view, window, |this, view, _: &Closed, window, cx| {
            this.remove(view, window, cx)
        })
        .detach();
        // The tab shows the title the shell sets.
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        self.tabs.insert((self.active + 1).min(self.tabs.len()), view);
        self.activate((self.active + 1).min(self.tabs.len() - 1), window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index;
        if let Some(view) = self.tabs.get(index) {
            view.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Closes a tab, ending its session.
    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.tabs.get(index).cloned() {
            view.update(cx, |view, _| view.terminate());
            self.remove(&view, window, cx);
        }
    }

    /// Takes view's tab away, its session having ended.
    fn remove(&mut self, view: &Entity<TerminalView>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab == view) else {
            return;
        };
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        let active = if index < self.active || self.active == self.tabs.len() {
            self.active - 1
        } else {
            self.active
        };
        self.activate(active, window, cx);
    }

    /// The machine of the tab you're on, for a new tab to open on.
    fn current_machine(&self, cx: &App) -> Option<Machine> {
        match self.tabs.get(self.active) {
            Some(view) => Some(view.read(cx).machine.clone()),
            None => self.machines.first().cloned(),
        }
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(machine) = self.current_machine(cx) {
            self.open(machine, window, cx);
        }
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.activate((self.active + 1) % self.tabs.len(), window, cx);
        }
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.activate((self.active + self.tabs.len() - 1) % self.tabs.len(), window, cx);
        }
    }

    /// Ends the sessions of every tab as the window goes, as closing them would.
    fn end_sessions(&mut self, cx: &mut App) {
        let ended: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|view| view.update(cx, |view, _| view.terminate()))
            .collect();
        // The app may be about to exit, so wait here for the daemon to hear of it.
        crate::runtime().block_on(async {
            let _ = tokio::time::timeout(QUIT_TIMEOUT, futures::future::join_all(ended)).await;
        });
    }

    fn render_titlebar(
        &mut self,
        tiling: Option<Tiling>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mac = cfg!(target_os = "macos");
        let tabs = self.tabs.iter().enumerate().map(|(index, view)| {
            let view = view.read(cx);
            let label: SharedString = match &view.title {
                Some(title) if !title.is_empty() => format!("{} · {title}", view.machine.label()).into(),
                _ => view.machine.label().to_owned().into(),
            };
            self.render_tab(index, label, cx)
        });
        let tabs: Vec<_> = tabs.collect();

        div()
            .id("titlebar")
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(TITLEBAR_HEIGHT)
            .w_full()
            .bg(theme::titlebar())
            .border_b_1()
            .border_color(theme::border())
            .when_some(tiling, |bar, tiling| rounded(bar, tiling, true, false))
            .when(mac, |bar| bar.pl(px(80.)))
            .window_control_area(WindowControlArea::Drag)
            // The window moves once the mouse drags it, so that a double click reaches the app.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = false))
            .on_mouse_down_out(cx.listener(|this, _, _, _| this.dragging = false))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if std::mem::take(&mut this.dragging) {
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
                        window.zoom_window();
                    }
                }
            })
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position)
            })
            .when(self.tabs.is_empty(), |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted())
                        .child("DeskConn Terminal"),
                )
            })
            // Tabs shrink to make room, and are cut off past that, rather than run under the buttons.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_shrink()
                    .h_full()
                    .items_end()
                    .min_w_0()
                    .overflow_hidden()
                    .children(tabs),
            )
            .child(
                button("new-tab", "+")
                    .ml_1()
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx))),
            )
            .child(
                button("machines", "▾")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = !this.menu;
                        cx.notify();
                    }))
                    .when(self.menu, |button| button.child(self.render_menu(cx))),
            )
            // Room to drag the window by, however many tabs there are.
            .child(div().flex_1().min_w(px(48.)))
            .when(!mac, |bar| bar.child(window_controls(window)))
    }

    fn render_tab(&self, index: usize, label: SharedString, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = index == self.active;
        div()
            .id(("tab", index))
            .group("tab")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(32.))
            .flex_shrink()
            .min_w(px(72.))
            .max_w(px(240.))
            .pl_3()
            .pr_1()
            .rounded_t_md()
            .text_sm()
            .when(active, |tab| {
                tab.bg(theme::tab_active()).text_color(theme::foreground())
            })
            .when(!active, |tab| {
                tab.text_color(theme::muted()).hover(|tab| tab.bg(theme::tab_hover()))
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.activate(index, window, cx)))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close(index, window, cx)),
            )
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(
                button(("close-tab", index), "×")
                    .invisible()
                    .group_hover("tab", |button| button.visible())
                    .when(active, |button| button.visible())
                    .on_click(cx.listener(move |this, _, window, cx| this.close(index, window, cx))),
            )
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let items = self.machines.iter().enumerate().map(|(index, machine)| {
            let machine = machine.clone();
            div()
                .id(("machine", index))
                .px_3()
                .py_1p5()
                .rounded_md()
                .text_sm()
                .text_color(theme::foreground())
                .hover(|item| item.bg(theme::tab_hover()))
                .child(machine.label().to_owned())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = false;
                    this.open(machine.clone(), window, cx);
                }))
        });
        deferred(
            anchored().child(
                div()
                    .id("machine-menu")
                    .occlude()
                    .mt(px(30.))
                    .min_w(px(200.))
                    .p_1()
                    .bg(theme::titlebar())
                    .border_1()
                    .border_color(theme::border())
                    .rounded_lg()
                    .shadow_lg()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = false;
                        cx.notify();
                    }))
                    .children(items),
            ),
        )
    }

    /// What the window shows without tabs: the machines to open one on, or why there are none.
    fn render_machines(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(theme::foreground());
        if let Some(error) = &self.error {
            return page
                .child(div().text_color(theme::error()).child(error.clone()))
                .into_any_element();
        }
        if self.machines.is_empty() {
            return page
                .child(
                    div()
                        .text_color(theme::muted())
                        .child("No machines yet. Sign in with `desk auth login`."),
                )
                .into_any_element();
        }
        page.child(div().text_xl().mb_2().child("Your machines"))
            .children(self.machines.iter().enumerate().map(|(index, machine)| {
                let machine = machine.clone();
                div()
                    .id(("open", index))
                    .w(px(320.))
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .bg(theme::tab_hover())
                    .hover(|item| item.bg(theme::border()))
                    .child(machine.label().to_owned())
                    .on_click(cx.listener(move |this, _, window, cx| this.open(machine.clone(), window, cx)))
            }))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tiling = match window.window_decorations() {
            Decorations::Client { tiling } => Some(tiling),
            Decorations::Server => None,
        };
        window.set_client_inset(if tiling.is_some() { SHADOW } else { px(0.) });

        let body = match self.tabs.get(self.active) {
            Some(view) => div().flex_1().min_h_0().child(view.clone()).into_any_element(),
            None => self.render_machines(cx),
        };
        let content = div()
            .id("workspace")
            .key_context("Workspace")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::background())
            .font_family(".SystemUIFont")
            .child(self.render_titlebar(tiling, window, cx))
            .child(body);
        let Some(tiling) = tiling else {
            return content.into_any_element();
        };

        // A window that draws its own decorations: rounded, with a border, and a shadow in the
        // room around it, but for the sides against a screen edge or another window.
        let content = rounded(content, tiling, true, true)
            .border_color(theme::border())
            .when(!tiling.top, |div| div.border_t_1())
            .when(!tiling.bottom, |div| div.border_b_1())
            .when(!tiling.left, |div| div.border_l_1())
            .when(!tiling.right, |div| div.border_r_1())
            .when(!tiling.is_tiled(), |div| {
                div.shadow(vec![BoxShadow {
                    color: hsla(0., 0., 0., 0.4),
                    offset: point(px(0.), px(0.)),
                    blur_radius: SHADOW / 2.,
                    spread_radius: px(0.),
                }])
            });
        div()
            .id("window")
            .relative()
            .size_full()
            .when(!tiling.top, |div| div.pt(SHADOW))
            .when(!tiling.bottom, |div| div.pb(SHADOW))
            .when(!tiling.left, |div| div.pl(SHADOW))
            .when(!tiling.right, |div| div.pr(SHADOW))
            .child(content)
            .child(resize_handles(tiling))
            .into_any_element()
    }
}

/// Rounds element's top or bottom corners, but for those against a screen edge or another
/// window.
fn rounded<E: Styled + FluentBuilder>(element: E, tiling: Tiling, top: bool, bottom: bool) -> E {
    element
        .when(top && !tiling.top && !tiling.left, |e| e.rounded_tl(ROUNDING))
        .when(top && !tiling.top && !tiling.right, |e| e.rounded_tr(ROUNDING))
        .when(bottom && !tiling.bottom && !tiling.left, |e| e.rounded_bl(ROUNDING))
        .when(bottom && !tiling.bottom && !tiling.right, |e| e.rounded_br(ROUNDING))
}

/// A small titlebar button.
fn button(id: impl Into<gpui::ElementId>, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(24.))
        .rounded_md()
        .text_color(theme::muted())
        .hover(|button| button.bg(theme::tab_hover()).text_color(theme::foreground()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

/// Minimize, maximize and close, for platforms that leave drawing them to the app.
fn window_controls(window: &Window) -> impl IntoElement {
    let control = |id: &'static str, label: &'static str, area| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(46.))
            .h_full()
            .text_color(theme::muted())
            .window_control_area(area)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(label)
    };
    div()
        .flex()
        .flex_row()
        .flex_none()
        .h_full()
        .child(
            control("minimize", "—", WindowControlArea::Min)
                .hover(|c| c.bg(theme::tab_hover()))
                .on_click(|_, window, _| window.minimize_window()),
        )
        .child(
            control(
                "maximize",
                if window.is_maximized() { "❐" } else { "□" },
                WindowControlArea::Max,
            )
            .hover(|c| c.bg(theme::tab_hover()))
            .on_click(|_, window, _| window.zoom_window()),
        )
        .child(
            control("close", "✕", WindowControlArea::Close)
                .hover(|c| c.bg(theme::close_hover()).text_color(theme::foreground()))
                .on_click(|_, window, _| window.remove_window()),
        )
}

/// Lets a window that draws its own decorations be resized from its edges: from the room around
/// it, or just inside its border. Each edge and corner has a hitbox of its own, so that the
/// pointer takes the right shape wherever it goes. Corners reach further, to be easier to grab,
/// and sides against a screen edge or another window don't resize.
fn resize_handles(tiling: Tiling) -> impl IntoElement {
    canvas(
        move |_, window, _| {
            let size = window.viewport_size();
            let side = SHADOW + RESIZE_INSIDE;
            let corner = side + RESIZE_CORNER;
            let (w, h) = (size.width, size.height);
            let area = |x, y, width, height| Bounds::new(point(x, y), gpui::size(width, height));
            let mut areas = Vec::new();
            if !tiling.top {
                areas.push((ResizeEdge::Top, area(px(0.), px(0.), w, side)));
            }
            if !tiling.bottom {
                areas.push((ResizeEdge::Bottom, area(px(0.), h - side, w, side)));
            }
            if !tiling.left {
                areas.push((ResizeEdge::Left, area(px(0.), px(0.), side, h)));
            }
            if !tiling.right {
                areas.push((ResizeEdge::Right, area(w - side, px(0.), side, h)));
            }
            // Last, so that they're over the sides.
            if !tiling.top && !tiling.left {
                areas.push((ResizeEdge::TopLeft, area(px(0.), px(0.), corner, corner)));
            }
            if !tiling.top && !tiling.right {
                areas.push((ResizeEdge::TopRight, area(w - corner, px(0.), corner, corner)));
            }
            if !tiling.bottom && !tiling.left {
                areas.push((ResizeEdge::BottomLeft, area(px(0.), h - corner, corner, corner)));
            }
            if !tiling.bottom && !tiling.right {
                areas.push((ResizeEdge::BottomRight, area(w - corner, h - corner, corner, corner)));
            }
            areas
                .into_iter()
                .map(|(edge, bounds)| (edge, window.insert_hitbox(bounds, HitboxBehavior::BlockMouse)))
                .collect::<Vec<_>>()
        },
        |_, handles, window, _| {
            for (edge, hitbox) in &handles {
                window.set_cursor_style(resize_cursor(*edge), hitbox);
            }
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                if let Some((edge, _)) = handles.iter().rev().find(|(_, hitbox)| hitbox.is_hovered(window)) {
                    window.start_window_resize(*edge);
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_full()
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}
