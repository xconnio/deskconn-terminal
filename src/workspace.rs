//! The window: tabs in the title bar, each a terminal on a machine.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, CursorStyle, Decorations, Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior,
    MouseButton, MouseDownEvent, Pixels, Point, ResizeEdge, SharedString, Size, Window, WindowControlArea, actions,
    anchored, canvas, deferred, div, prelude::*, px,
};

use crate::daemon::{self, Machine};
use crate::terminal_view::{Closed, TerminalView};
use crate::theme;

actions!(workspace, [NewTab, CloseTab, NextTab, PreviousTab]);

const TITLEBAR_HEIGHT: Pixels = px(38.);
/// How far from a client-decorated window's edge it can be resized.
const RESIZE_BORDER: Pixels = px(6.);
/// How long quitting waits for the sessions to end.
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
        cx.on_app_quit(Self::quit).detach();
        workspace
    }

    fn open(&mut self, machine: Machine, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir.clone();
        let view = cx.new(|cx| TerminalView::new(machine, dir, cx));
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

    /// Ends the sessions of every tab before the app quits, as closing them would.
    fn quit(&mut self, cx: &mut Context<Self>) -> impl Future<Output = ()> + use<> {
        let ended: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|view| view.update(cx, |view, _| view.terminate()))
            .collect();
        let timeout = cx.background_executor().timer(QUIT_TIMEOUT);
        async move {
            let ended = futures::future::join_all(ended);
            futures::future::select(std::pin::pin!(ended), timeout).await;
        }
    }

    fn render_titlebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mac = cfg!(target_os = "macos");
        let tabs = self.tabs.iter().enumerate().map(|(index, view)| {
            let view = view.read(cx);
            let label: SharedString = match &view.title {
                Some(title) if !title.is_empty() => format!("{} · {title}", view.machine.label()).into(),
                _ => view.machine.label().to_owned().into(),
            };
            self.render_tab(index, label, cx)
        });

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
            .when(mac, |bar| bar.pl(px(80.)))
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position)
            })
            .when(self.tabs.is_empty(), |bar| {
                bar.child(
                    div()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted())
                        .child("DeskConn Terminal"),
                )
            })
            .child(div().flex().flex_row().h_full().items_end().min_w_0().children(tabs))
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
            .child(div().flex_1())
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
            .bg(theme::background())
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
        let decorations = window.window_decorations();
        let body = match self.tabs.get(self.active) {
            Some(view) => div().flex_1().min_h_0().child(view.clone()).into_any_element(),
            None => self.render_machines(cx),
        };
        div()
            .id("workspace")
            .key_context("Workspace")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::background())
            .font_family(".SystemUIFont")
            .child(self.render_titlebar(window, cx))
            .child(body)
            .when(matches!(decorations, Decorations::Client { .. }), |root| {
                root.child(resize_handles())
            })
    }
}

/// A small titlebar button.
fn button(id: impl Into<gpui::ElementId>, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
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

/// Lets a window that draws its own decorations be resized from its edges.
fn resize_handles() -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        |_, hitbox: Hitbox, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(window.mouse_position(), size) {
                window.set_cursor_style(resize_cursor(edge), &hitbox);
            }
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                if let Some(edge) = resize_edge(event.position, window.window_bounds().get_bounds().size) {
                    window.start_window_resize(edge);
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_full()
}

fn resize_edge(position: Point<Pixels>, size: Size<Pixels>) -> Option<ResizeEdge> {
    let near = |v: Pixels, max: Pixels| (v < RESIZE_BORDER, v > max - RESIZE_BORDER);
    let (left, right) = near(position.x, size.width);
    let (top, bottom) = near(position.y, size.height);
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}
