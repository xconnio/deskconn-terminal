//! DeskConn Terminal: terminals on your DeskConn machines, through the local deskconn daemon.

mod daemon;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod desktop;
mod keys;
mod links;
mod palette;
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

use crate::terminal_view::{Clear, Copy, DecreaseFontSize, IncreaseFontSize, Paste, Reset, ResetFontSize};
use crate::workspace::{ActivateTab, CloseTab, NewTab, NewTabOn, NextTab, PreviousTab, Quit, Workspace};

/// The window's app ID, by which a desktop knows the app, and finds its icon.
const APP_ID: &str = "io.xconn.deskconn.terminal";

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
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Err(err) = desktop::register() {
        eprintln!("register with the desktop: {err:#}");
    }
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
            app_id: Some(APP_ID.into()),
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
            KeyBinding::new("cmd-shift-k", NewTabOn, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-shift-]", NextTab, None),
            KeyBinding::new("cmd-shift-[", PreviousTab, None),
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-k", Clear, Some("Terminal")),
            KeyBinding::new("cmd-alt-k", Reset, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("cmd-=", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd-+", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd--", DecreaseFontSize, Some("Terminal")),
            KeyBinding::new("cmd-0", ResetFontSize, Some("Terminal")),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-t", NewTab, None),
            KeyBinding::new("ctrl-shift-k", NewTabOn, None),
            // Only once the last tab is closed, as programs in a terminal may take Ctrl+Q.
            KeyBinding::new("ctrl-q", Quit, Some("Home")),
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
            KeyBinding::new("ctrl-shift-l", Clear, Some("Terminal")),
            KeyBinding::new("ctrl-shift-alt-l", Reset, Some("Terminal")),
            KeyBinding::new("ctrl-=", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl-+", IncreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl--", DecreaseFontSize, Some("Terminal")),
            KeyBinding::new("ctrl-0", ResetFontSize, Some("Terminal")),
        ]);
    }

    // The first tabs by number: Alt+1 to Alt+9, and Alt+0 for the tenth; on macOS, Cmd+1 to
    // Cmd+9, as Cmd+0 resets the font size there.
    let (modifier, tabs) = if cfg!(target_os = "macos") {
        ("cmd", 9)
    } else {
        ("alt", 10)
    };
    cx.bind_keys(
        (0..tabs).map(|index| KeyBinding::new(&format!("{modifier}-{}", (index + 1) % 10), ActivateTab(index), None)),
    );
}
