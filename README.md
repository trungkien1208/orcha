<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/banner-dark.svg" />
    <img src="docs/orcha/images/banner-light.svg" alt="orcha: every agent, every project, in rhythm. A calm terminal workspace for Claude, Codex and Cursor." width="100%" />
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

<p align="center"><b>Claude Code, Codex and Cursor across all your projects, in one terminal.</b><br/>See who's working, who's waiting on you, and how much fuel is left, without hunting through tabs.</p>

## Install

**macOS** (Apple Silicon)

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

Then run `orcha` inside a project. Your agent sign-ins are picked up automatically, and the first run offers to import projects from Orca, VS Code, Cursor and JetBrains.

## Why orcha

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/features-dark.svg" />
    <img src="docs/orcha/images/features-light.svg" alt="Projects together · Live status · Fuel and accounts · Continue on · Git in view · Bring your projects" width="100%" />
  </picture>
</p>

- **Every agent at a glance.** Each project lists its tabs and agents; a folded project still shows `running` or `needs you` with the agents involved.
- **Never stall on limits.** Fuel gauges per account. When one runs low, **CONTINUE ON** moves the chat to another account or hands it to a different agent, whole or summarized.
- **Paths you can click.** Ctrl+click anything an agent prints: folders open in Finder/Explorer, files in the app you pick once per type, at the right line.
- **Git beside the work.** Branch, changes and history in the side panel, one click to your IDE or Git app.
- **Agents outlive the window.** Quit orcha and they keep running; `orcha` brings you straight back.

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

Best in [Ghostty](https://ghostty.org) on macOS and [WezTerm](https://wezfurlong.org/wezterm/) on Windows. The agent icon font, and font, size, and ligature settings, still apply in Ghostty only. On WezTerm the logos stay plain symbols, and the font is whatever WezTerm is set to. Using herdr already? Your config and workspaces come along, and herdr stays untouched.

This is an early build. If something breaks, [open an issue](https://github.com/trungkien1208/orcha/issues) with a screenshot.

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

orcha is a fork of [herdr](https://github.com/herdrdev/herdr). herdr does the hard part: the terminal runtime, sessions and agent detection. orcha adds the look and the agent workflow on top. herdr's own README is in [docs/orcha/herdr-README.md](docs/orcha/herdr-README.md).

Licensed under the [Apache License 2.0](LICENSE), like herdr.
