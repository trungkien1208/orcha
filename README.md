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

<p align="center"><b>All your agents. One terminal. Zero tab chaos.</b><br/>Run Claude Code, Codex and Cursor side by side and see at a glance who's working and who needs you.<br/>The best ideas from Orca, brought to the terminal.</p>

<p align="center"><a href="https://github.com/trungkien1208/orcha/releases/tag/orcha-0.9.9"><b>0.9.9 beta · what's new</b></a></p>

## Install

**macOS** (Apple Silicon or Intel). Paste this into Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
```

**Windows** (x64). Paste this into PowerShell:

```powershell
irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex
```

<details>
<summary>Using cmd instead of PowerShell?</summary>

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex"
```

</details>

## Get started

1. Open your terminal in a project folder.
2. Type `orcha` and press Enter.
3. That's it. orcha finds the agents you're already signed in to, and the first time it offers to bring in your projects from Orca, VS Code, Cursor or JetBrains.

Any terminal works. For the nicest look, use [Ghostty](https://ghostty.org) on macOS or [WezTerm](https://wezfurlong.org/wezterm/) on Windows.

## What you get

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/features-dark.svg" />
    <img src="docs/orcha/images/features-light.svg" alt="Projects together · Live status · Fuel and accounts · Continue on · Git in view · Bring your projects" width="100%" />
  </picture>
</p>

- **Everything in one place.** Your projects and their agents sit in one sidebar. Each agent shows `running`, `needs you` or `done`, so nobody waits in silence.
- **Never run out mid-task.** The fuel gauge shows how much of each account's plan is left. When it gets low, **CONTINUE ON** moves the chat to another account or another agent, with a short summary so it can pick up where it left off.
- **Close it, they keep going.** Quit orcha and your agents keep working. Type `orcha` again to come back. (A restart of your computer does stop them.)
- **Click to open.** Ctrl+click a file or folder an agent mentions and it opens in your editor or file manager.
- **Git at a glance.** See your branch and changed files on the side, and open them in your editor or Git app with one click.
- **Find any action.** Click `⌕ commands` at the top and type what you want: new agent, new branch, ask every agent, open in editor.
- **Make it yours.** Light and dark themes, colors, layout and an emoji for each project. Open Settings, then `look`.

<table>
  <tr>
    <td width="50%"><img src="docs/orcha/images/continue-on.png" alt="CONTINUE ON: accounts with their fuel, the best next one marked" /></td>
    <td width="50%"><img src="docs/orcha/images/open-with.png" alt="OPEN WITH: pick the app for a file type once" /></td>
  </tr>
  <tr>
    <td align="center"><sub><b>CONTINUE ON</b> · switch account or agent when one runs low</sub></td>
    <td align="center"><sub><b>OPEN WITH</b> · pick which app opens each file type</sub></td>
  </tr>
</table>

## Commands

```sh
orcha             # open orcha (or come back to it)
orcha update      # get the latest version
orcha uninstall   # remove orcha (asks before touching your settings)
```

orcha also shows a green button when an update is ready.

## Good to know

- Already use herdr? Your settings and workspaces come along, and herdr stays as it is.
- Agent logos show up in Ghostty, WezTerm and iTerm2. Other terminals show simple symbols instead.
- Something looks wrong? [Open an issue](https://github.com/trungkien1208/orcha/issues) with your OS, terminal, orcha version and a screenshot. Beta feedback shapes 1.0.

<details>
<summary><b>Updating from an older version</b></summary>

Run `orcha update`, then quit orcha and open it again.

On macOS, if you see `config.toml has unknown keys` after updating, the old version is still running in the background. Switch it to the new one with:

```sh
orcha server live-handoff --import-exe ~/.local/bin/orcha
```

Then open orcha again. Your running agents move over without stopping. Avoid `orcha server stop` while agents are busy, because that stops them too.

</details>

<details>
<summary><b>For tinkerers</b></summary>

- Your own color palette: add a style file at `~/.config/orcha/styles/<name>.toml` (macOS).
- Font, size and ligature settings apply in Ghostty only.
- The macOS installer uses Python 3 for a few extras, such as hooks and the icon font.

</details>

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

orcha is built on [herdr](https://github.com/herdrdev/herdr), which does the heavy lifting: running the terminals, keeping sessions alive and spotting which agent is which. orcha adds the look and the agent workflow on top, with plenty of ideas borrowed from Orca. herdr's own README is in [docs/orcha/herdr-README.md](docs/orcha/herdr-README.md).

Licensed under the [Apache License 2.0](LICENSE), like herdr.
