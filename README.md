# DeskConn Terminal

A terminal for your DeskConn machines, built with [GPUI](https://www.gpui.rs) and
[alacritty_terminal](https://crates.io/crates/alacritty_terminal).

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

## Keys

| Linux and Windows                    | macOS                       | Action           |
|--------------------------------------|-----------------------------|------------------|
| Ctrl+Shift+T                         | Cmd+T                       | New tab          |
| Ctrl+Shift+W                         | Cmd+W                       | Close tab        |
| Ctrl+Tab, Ctrl+Page Down             | Cmd+Shift+]                 | Next tab         |
| Ctrl+Shift+Tab, Ctrl+Page Up         | Cmd+Shift+[                 | Previous tab     |
| Ctrl+Shift+V, Shift+Insert           | Cmd+V                       | Paste            |

`+` opens a tab on the machine of the current one; `▾` picks another machine.

## Notes

GPUI comes from Zed's repository, at a release tag. Its `zed-async-tar` needs `xattr` 0.2,
which no longer builds with current `libc`; `vendor/xattr` is that crate with the one-line fix,
used through `[patch.crates-io]` until GPUI moves on.
