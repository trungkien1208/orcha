# Running ORCHA from source

On macOS or Linux, from the checkout:

```sh
bash scripts/dev-orcha.sh
```

The launcher sets `ORCHA_APP=1`, starts in the checkout directory, clears inherited
socket and pane overrides, and selects the named `orcha-dev` session. It keeps the
normal nested-session permission check. Cargo builds only what changed.

For the equivalent command without the launcher:

```sh
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH ORCHA_APP=1 cargo run -- --session orcha-dev
```

Running `cargo run` without `ORCHA_APP=1` starts the Herdr identity instead, with a
different config directory. The ORCHA welcome and setup menu belong to the ORCHA
identity. Welcome appears once; use **menu → setup** to reopen it.

## Creating a project workspace

**+ NEW** offers a workspace in the current project, a native folder browser,
and manual folder path entry. The browser runs in the background and fills a
confirmation prompt; Enter creates the workspace, Escape cancels. macOS uses its
system folder chooser, Windows uses its folder browser, and Linux uses Zenity
when available. If the dialog is unavailable, enter the path manually. Remote
projects use a path on the remote machine, not a locally selected folder.

Automatically named home workspaces display their folder name and workspace
number instead of repeated `~` labels. Custom names remain unchanged.

## Project sidebar

Projects use a continuous list with an indented row for each pane. The selected
project's Git branch appears below its name. Click the disclosure arrow to fold
or expand any project, including the selected one.

Agent rows use their provider logo when the terminal supports the installed icon
font, or a standard symbol otherwise. Working agents use their provider color;
idle icons are muted and agents waiting for input are red. A folded project shows
`running`, `needs you`, `done`, or `idle` with the relevant agent icons. When any
agents are active, their icons take priority over idle agents.

The activity footer starts as a one-line summary. Click it to expand the process
details or fold them again. Folded process details do not animate or request
animation repaints. With no active agents, the footer reads `No agents running`.

## Startup status timeout

The startup compatibility probe uses a response deadline and retries once when
the status API is temporarily unavailable. It never replaces a listening server
automatically. To inspect an existing session without stopping its agents:

```sh
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH ORCHA_APP=1 \
    target/debug/herdr --session orcha-dev status
```
