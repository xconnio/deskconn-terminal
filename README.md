# DeskConn Terminal

A terminal for your DeskConn machines, built with [GPUI](https://www.gpui.rs) and
[libghostty-vt](https://ghostty.org) (through the [ghostty-vt](https://crates.io/crates/ghostty-vt) crate).

It connects through the local deskconn daemon (`desk serve`), as the `desk` CLI does, and
opens the machines the CLI knows about (`desk ls`). Each tab is a persistent session on its
machine: a lost connection is restored without losing the shell, and closing the tab ends it.

```sh
make run      # build and run
make test     # unit tests
make check    # formatting and lints
make release  # optimized build in target/release/
make snap     # snap package, with snapcraft
```

## Requirements

- A signed-in `desk` CLI with its daemon running (`desk auth login`, then `desk serve`).
- On Linux, the development packages GPUI needs. On Debian and Ubuntu:

  ```sh
  sudo apt install build-essential clang cmake pkg-config libxkbcommon-dev \
    libxkbcommon-x11-dev libwayland-dev libxcb1-dev libx11-xcb-dev libvulkan-dev \
    libfontconfig-dev libfreetype-dev libzstd-dev
  ```

- On Linux, [Zig](https://ziglang.org/download/) 0.17 to build libghostty-vt from source (set
  `ZIG` if it isn't on your `PATH`). macOS uses Ghostty's prebuilt library. Either way, it's
  linked statically, so the app needs nothing else to run.

## Keys

| Linux                        | macOS         | Action             |
|------------------------------|---------------|--------------------|
| Ctrl+Shift+T                 | Cmd+T         | New tab            |
| Ctrl+Shift+K                 | Cmd+Shift+K   | New tab on…        |
| Ctrl+Shift+W                 | Cmd+W         | Close tab          |
| Ctrl+Q, with no tabs open    | Cmd+Q         | Quit               |
| Ctrl+Tab, Ctrl+Page Down     | Cmd+Shift+]   | Next tab           |
| Ctrl+Shift+Tab, Ctrl+Page Up | Cmd+Shift+[   | Previous tab       |
| Alt+1 … Alt+9, Alt+0         | Cmd+1 … Cmd+9 | Tab 1 to 9, tab 10 |
| Ctrl+Shift+C, Ctrl+Insert    | Cmd+C         | Copy               |
| Ctrl+Shift+V, Shift+Insert   | Cmd+V         | Paste              |
| Ctrl+Shift+A                 | Cmd+A         | Select all         |
| Ctrl+Shift+L                 | Cmd+K         | Clear              |
| Ctrl+Shift+Alt+L             | Cmd+Alt+K     | Reset              |
| Ctrl+=                       | Cmd+=         | Larger font        |
| Ctrl+-                       | Cmd+-         | Smaller font       |
| Ctrl+0                       | Cmd+0         | Default font size  |

On Linux, selected text can be pasted with a middle click. Ctrl+click (Cmd+click on macOS) opens
a link, and a right click opens a menu to copy, paste, select all, clear, or reset.

Programs that use the mouse, such as Claude Code, Codex, vim, and htop, receive its clicks, drags,
and wheel. Hold Shift to select text or open the menu instead.

Clear removes the scrollback and moves the prompt to the top. Reset restores the terminal to its
initial state, which helps when a program has left it garbled.

`+` opens a tab on the current tab's machine. To open one on another machine, pick it from `▾`, or
press Ctrl+Shift+K and type part of its name.

## Notes

GPUI comes from Zed's repository at a release tag. Its dependency `zed-async-tar` needs `xattr`
0.2, which no longer builds with current `libc`. `vendor/xattr` is that crate with a one-line fix,
applied through `[patch.crates-io]` until GPUI moves on.

The terminal font, [JetBrains Mono](https://www.jetbrains.com/lp/mono/), is bundled from
`assets/fonts` under the SIL Open Font License (`assets/fonts/OFL.txt`).

The snap needs access to the `desk` CLI's configuration and daemon in `~/.deskconn`, which it's
given with `sudo snap connect deskconn-terminal:deskconn`.

On Linux, the app registers itself with the desktop on startup, unless it's a snap. It writes an entry to
`~/.local/share/applications` and its icon to `~/.local/share/icons`, both named after its app ID,
`io.xconn.deskconn.terminal`. The desktop needs these to show the window's icon, and the entry
launches the executable from wherever it was last run.
