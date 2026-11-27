//! DeskConn Terminal: terminals on your DeskConn machines, through the local deskconn daemon.

mod daemon;
mod keys;
mod links;
mod protocol;
mod selection;
mod session;
mod settings;
mod terminal_view;
mod theme;
mod workspace;

use std::sync::LazyLock;

use gpui::{
    App, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions, point, prelude::*, px,
    size,
};

use crate::terminal_view::{ClearScrollback, Copy, DecreaseFontSize, IncreaseFontSize, Paste, ResetFontSize};
use crate::workspace::{CloseTab, NewTab, NextTab, PreviousTab, Workspace};

/// The runtime the sessions' connections run on.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("start the async runtime")
    });
    &RUNTIME
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(terminal_view::fonts())
            .expect("load the terminal's font");
        cx.set_global(settings::Settings::load());
        bind_keys(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("DeskConn Terminal".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.), px(12.))),
            }),
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("io.xconn.deskconn.terminal".into()),
            window_min_size: Some(size(px(360.), px(200.))),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(window, cx)))
            .expect("open the window");
        cx.activate(true);
    });
}

fn bind_keys(cx: &mut App) {
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-t", NewTab, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PreviousTab, None),
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-k", ClearScrollback, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-=", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd-+", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd--", DecreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd-0", ResetFontSize, Some("Terminal")),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-t", NewTab, None),
            KeyBinding::new("ctrl-shift-w", CloseTab, None),
            KeyBinding::new("ctrl-pagedown", NextTab, None),
            KeyBinding::new("ctrl-pageup", PreviousTab, None),
            KeyBinding::new("ctrl-tab", NextTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
            // The later bindings of an action are the ones menus show.
            KeyBinding::new("ctrl-insert", Copy, Some("Terminal")),
            KeyBinding::new("shift-insert", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-shift-c", Copy, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-=", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl-+", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl--", DecreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl-0", ResetFontSize, Some("Terminal")),
        ]);
    }
}
