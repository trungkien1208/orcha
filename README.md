<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/banner-dark.svg" />
    <img src="docs/orcha/images/banner-light.svg" alt="orcha: all your agents, one terminal, zero tab chaos. Claude Code, Codex and Cursor side by side, with a pixel whale saying Codex needs you." width="100%" />
  </picture>
</p>

<p align="center">
  <a href="#install"><b>Install</b></a> ·
  <a href="https://trungkien1208.github.io/orcha/">Website</a> ·
  <a href="#why-orcha">Why orcha</a> ·
  <a href="#commands">Commands</a> ·
  <a href="https://github.com/trungkien1208/orcha/issues">Feedback</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/dark.png" />
    <img src="docs/orcha/images/light.png" alt="orcha: projects on the left, two agents side by side, fuel and git on the right" width="100%" />
  </picture>
</p>

<p align="center"><b>All your agents. One terminal. Zero tab chaos.</b><br/>Claude Code, Codex and Cursor work side by side, and you can see who's cooking and who's stuck.<br/>We loved how Orca runs a whole crew of agents at once, so we moved the good parts into the terminal.</p>

<p align="center"><a href="https://github.com/trungkien1208/orcha/releases/tag/orcha-0.9.9"><b>0.9.9 beta · what's new</b></a><br/>New settings, emoji for your projects, and one search box for every command.</p>

## Install

**macOS** (Apple Silicon or Intel)

```sh
curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
```

**Windows** (x64, PowerShell). Use [WezTerm](https://wezfurlong.org/wezterm/).

```powershell
irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex
```

<details>
<summary>From cmd instead</summary>

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex"
```

</details>

Then run `orcha` inside a project. It finds your agent sign-ins on its own, and the first run offers to bring your projects over from Orca, VS Code, Cursor and JetBrains.

You don't need Ghostty. orcha ships its own terminal engine, so your usual terminal is fine. Fuel, accounts and hand-off work inside the app on both macOS and Windows. The macOS installer still uses Python 3 for a few extras, like hooks and the icon font.

## Why orcha

Orca showed how nice it is to run a pile of coding agents in parallel. Most of those agents live in a terminal anyway, so orcha brings that workflow to them instead of the other way round: projects, worktrees, live status and hand-offs, all in one window you can quit without stopping anything.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/features-dark.svg" />
    <img src="docs/orcha/images/features-light.svg" alt="Projects together · Live status · Fuel and accounts · Continue on · Git in view · Bring your projects" width="100%" />
  </picture>
</p>

- See every agent at a glance. Each project lists its tabs and agents, and a folded project still shows `running` or `needs you` along with who's involved. Nobody waits in silence.
- Running low on fuel? Check usage per account, then pick **CONTINUE ON** to switch accounts or hand the job to another agent. The hand-off brings notes on the goal, the recent conversation and the changed files, and nothing is sent until you say so.
- Click the paths. Ctrl+click anything an agent prints: folders open in Finder or Explorer, and files open at the right line in the app you picked for that file type.
- Git sits beside the work. Branch, changes and history live in the side panel, one click away from your IDE or Git app.
- Close the window and let them cook. Quit orcha and your agents keep working in the background. Type `orcha` to pop back in. (A reboot still stops them. We're good, not magic.)
- One search box for everything. `⌕ commands` in the header, or the prefix key and then `:`, finds any action: new agent, worktree, hand off, ask every agent, open in IDE.
- Move the furniture around. In Settings → `look` you can put projects on either side, move or hide PROCESS, choose where FUEL sits, and tune colors and sidebar width. A little layout sketch shows what goes where. Right-click a project to give it its own emoji.
- Give an agent its own branch and folder with a fresh worktree from **NEW**, or use **Ask every agent here** to send one prompt to every agent in the project.
- Too many accounts? Open accounts and press `r` to refresh fuel or `d` to drop one from orcha. Its sign-in and chat history stay put.

Light and dark themes come in the box. Want your own palette? Add a style to `~/.config/orcha/styles/<name>.toml` on macOS.

<table>
  <tr>
    <td width="50%"><img src="docs/orcha/images/continue-on.png" alt="CONTINUE ON: accounts with their fuel, the best next one marked" /></td>
    <td width="50%"><img src="docs/orcha/images/open-with.png" alt="OPEN WITH: pick the app for a file type once" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>CONTINUE ON</b> · the chat moves on when an account runs low</sub></td>
    <td align="center"><sub><b>OPEN WITH</b> · choose an app per file type, once</sub></td>
  </tr>
</table>

## Commands

```sh
orcha             # open (or come back to) your workspace
orcha update      # latest version; a green button also tells you when one is out
orcha uninstall   # remove orcha and what it added; asks before touching your settings
```

### Coming from an older version?

Run `orcha update`, then quit the interface and open it again. 0.9.9 is a public beta and comes through the regular installer and updater.

On macOS, if you see `config.toml has unknown keys` after upgrading, the background server may still be running the old version. Move it to the newly installed binary with:

```sh
orcha server live-handoff --import-exe ~/.local/bin/orcha
```

Once it succeeds, reopen the interface. Live handoff moves your running panes to the new server. Avoid `orcha server stop` while agents are busy, because that stops them too.

orcha looks best in [Ghostty](https://ghostty.org) on macOS and [WezTerm](https://wezfurlong.org/wezterm/) on Windows, but any terminal works. Agent logos come from the icon font in Ghostty, WezTerm and iTerm2 when it's installed, and plain symbols show up elsewhere (change it in Settings → `look` → agent logos). Font, size and ligature settings only apply in Ghostty. Everywhere else you get your terminal's font. Already using herdr? Your config and workspaces come along, and herdr stays untouched.

Something looking wonky? [Send a bug report](https://github.com/trungkien1208/orcha/issues) with your OS, terminal, orcha version and a screenshot. Beta feedback shapes 1.0.

<details>
<summary><b>Build from source</b></summary>

You need Rust and [Zig 0.16](https://ziglang.org/download/).

```sh
git clone https://github.com/trungkien1208/orcha && cd orcha
ZIG=/path/to/zig cargo build --release
cp target/release/herdr ~/.local/bin/orcha   # the name "orcha" turns on the orcha app
```

</details>

## Thanks

orcha is a fork of [herdr](https://github.com/herdrdev/herdr), which does the heavy lifting: the terminal runtime, sessions and agent detection. orcha adds the look and the agent workflow on top, with plenty of ideas borrowed from Orca. herdr's own README is in [docs/orcha/herdr-README.md](docs/orcha/herdr-README.md).

Licensed under the [Apache License 2.0](LICENSE), like herdr.
