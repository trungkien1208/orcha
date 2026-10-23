<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/banner-dark.svg" />
    <img src="docs/orcha/images/banner-light.svg" alt="orcha: every agent, every project, in rhythm. A calm terminal workspace for Claude, Codex and Cursor." width="100%" />
  </picture>
</p>

<p align="center">
  <a href="#install"><b>Install</b></a> ·
  <a href="https://trungkien1208.github.io/orcha/">Website</a> ·
  <a href="#everyday-use">Everyday use</a> ·
  <a href="#update-and-uninstall">Update &amp; uninstall</a> ·
  <a href="https://github.com/trungkien1208/orcha/issues">Feedback</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/dark.png" />
    <img src="docs/orcha/images/light.png" alt="orcha: projects on the left, two agents side by side, fuel and git on the right" width="100%" />
  </picture>
</p>

## Install

Runs on **macOS** (Apple Silicon) and **Windows** (x64). One line, no questions.

**macOS**, in Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
```

**Windows**, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex
```

or in Command Prompt (cmd):

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1 | iex"
```

Then open a new terminal window **inside a project** and run:

```sh
orcha
```

That project opens by itself. The first time, orcha also offers to bring in the projects it finds from **Orca, VS Code, Cursor and JetBrains**, and it finds your **Claude Code, Codex and Cursor** sign-ins on its own. Nothing to set up.

<sub>On macOS, best in <a href="https://ghostty.org">Ghostty</a> (agent logos, theme, font). On Windows, use <a href="https://aka.ms/terminal">Windows Terminal</a>. Other terminals work too, with plain symbols. Already use herdr? Your herdr config and workspaces come along; herdr itself stays untouched.</sub>

## What you get

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/orcha/images/features-dark.svg" />
    <img src="docs/orcha/images/features-light.svg" alt="Projects together · Live status · Fuel and accounts · Continue on · Git in view · Bring your projects" width="100%" />
  </picture>
</p>

When an account runs low, **CONTINUE ON** picks the best next place for the chat: another account of the same agent, or a hand-off to Codex, Claude or Cursor (the whole conversation or a summary).

<p align="center">
  <img src="docs/orcha/images/continue-on.png" alt="the CONTINUE ON popup: accounts with their fuel, the best one marked" width="80%" />
</p>

## Everyday use

| | |
|---|---|
| **New agent** | `+ NEW`, pick Claude, Codex or Cursor and the account |
| **Settings** | `⚙ settings` top right: theme, font, IDE, Git app |
| **Open a file** | Ctrl+click any path an agent prints |
| **IDE / Git app** | the `↗ IDE` / `↗ Git` buttons in the right panel |
| **Import projects** | `≡ menu` → import projects |
| **Quit** | `⏻ quit` top right, or `≡ menu` → quit to terminal. Your agents keep running; `orcha` brings you back |

## Update and uninstall

```sh
orcha update      # the latest version (orcha also shows a green button when one is out)
orcha uninstall   # removes orcha and everything it added; asks before deleting your settings
orcha --version
```

## Feedback

This is an early build for friends to try. Something looks off or breaks? [Open an issue](https://github.com/trungkien1208/orcha/issues) with a screenshot. Ideas are welcome too.

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
