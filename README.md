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
```

## Requirements

- A signed-in `desk` CLI with its daemon running (`desk auth login`, then `desk serve`).
- On Linux, the development packages GPUI builds against, on Debian and Ubuntu:

  ```sh
  sudo apt install build-essential clang cmake pkg-config libxkbcommon-dev \
    libxkbcommon-x11-dev libwayland-dev libxcb1-dev libx11-xcb-dev libvulkan-dev \
    libfontconfig-dev libfreetype-dev libzstd-dev
  ```

- On Linux, [Zig](https://ziglang.org/download/) 0.17, which builds libghostty-vt from source
  (set `ZIG` if it isn't on the `PATH`). macOS uses Ghostty's prebuilt library. Either way it's
  linked statically, so the app needs nothing extra to run.

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

Selecting text with the mouse offers it to a middle click on Linux, and Ctrl+click (Cmd+click on
macOS) opens a link. A right click opens a menu to copy, paste, select all, clear, or reset.

Programs that take the mouse, such as Claude Code, Codex, vim, or htop, get its wheel, clicks, and
drags; holding Shift keeps them for the terminal, to select text or open the menu.

Clear forgets the scrollback and brings the prompt to the top; Reset puts the terminal back as it
started, for when a program has left it in a bad state.

`+` opens a tab on the machine of the current one; `▾` picks another machine, as does
Ctrl+Shift+K, by typing part of its name.

## Notes

GPUI comes from Zed's repository, at a release tag. Its `zed-async-tar` needs `xattr` 0.2,
which no longer builds with current `libc`; `vendor/xattr` is that crate with the one-line fix,
used through `[patch.crates-io]` until GPUI moves on.

The terminal's font, [JetBrains Mono](https://www.jetbrains.com/lp/mono/), is built into the
app from `assets/fonts`, under the SIL Open Font License (`assets/fonts/OFL.txt`).

On Linux, the app adds itself to the desktop as it starts: an entry in
`~/.local/share/applications` and its icon in `~/.local/share/icons`, named after its app ID,
`io.xconn.deskconn.terminal`. That's how the desktop shows the window's icon, and the entry starts
the executable wherever it was last run from.
