# DeskConn Terminal

Standalone terminal for your DeskConn machines. The terminal itself is
deskconn-web-app's `TerminalPanel`, installed from GitHub as an npm dependency.

It shares the deskconn CLI's sign-in (`~/.deskconn/id_ed25519`): if you're
logged in with `deskconn auth login` it opens straight to your machines, and
signing in here (when the key is missing, expired or revoked) logs the CLI in too.

```sh
make setup            # .env.example -> .env.local (if missing), npm install (deskconn-web pinned to a commit)
make run              # dev, with hot reload (make run-local: a hub on localhost)
make build            # type-check, build, AppImage/deb/dmg/exe in release/
```

It connects to the hub set in `.env.local` (the hosted one by default); change
`VITE_WAMP_WT_URL` / `VITE_WAMP_WT_CERT_URL` there for another one.
