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

**+ NEW** (and the tab strip's `+`) opens NEW AGENT: every account with its
fuel, a plain shell, and **New project…**. With no project yet it goes straight
to NEW PROJECT. NEW PROJECT offers another project in the current folder, a
native folder browser, manual folder path entry and a new worktree. The browser runs in the background and fills a
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

## Trying a build on your own orcha before a release

`scripts/dev-orcha.sh` runs a separate `orcha-dev` session with its own
config, so it never touches your everyday orcha. To live on a build for a
while before releasing it, put it in place of the installed one instead:

```sh
# 1. build from the working tree (one or two cargo builds at a time on 16 GB)
env -u ORCHA_APP -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH \
    CARGO_BUILD_JOBS=2 cargo build --release --locked

# 2. keep the installed one, then swap in the build (copy, then rename:
#    a new file, so the running orcha and macOS's signature check are safe)
cp ~/.local/bin/orcha ~/.local/bin/orcha.before-test
cp target/release/herdr ~/.local/bin/orcha.new
mv ~/.local/bin/orcha.new ~/.local/bin/orcha

# 3. move the background server to the new binary without stopping agents,
#    then open orcha again
orcha server live-handoff --import-exe ~/.local/bin/orcha
orcha
```

The build reports the version in `src/orcha_app.rs` (`ORCHA_VERSION`), so
`orcha --version` looks unchanged until the release bumps it. Features that
need new data from the server (for example pane memory) only show up after
step 3.

To go back:

```sh
mv ~/.local/bin/orcha.before-test ~/.local/bin/orcha
orcha server live-handoff --import-exe ~/.local/bin/orcha
```

If the hand-off fails, `orcha server stop` and `orcha` start a fresh server
on the binary in place. That stops running agents; they resume from their
saved sessions where the agent supports it.

## Cutting a release

```sh
just orcha-release 0.14.0
```

It needs a clean tree on `orcha`, in sync with origin. It writes
`docs/orcha/releases/<version>.md` from the commit subjects when missing and
opens it to tidy, sets `ORCHA_VERSION`, commits, tags `orcha-<version>`,
pushes and follows the GitHub build (Mac arm64 + x86_64, Windows zip). Flags:
`--test` also runs the tests, `--no-watch` skips following CI.

The Mac jobs often get cancelled for runner capacity. Re-run only those:

```sh
gh run rerun <run-id> --failed
```
