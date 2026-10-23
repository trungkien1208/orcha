<p align="center">
  <img src="assets/orcha-mark.svg" alt="orcha: a whale in a matcha bowl" width="96" />
</p>

<h1 align="center">orcha</h1>

<p align="center">
  <b>Every agent. Every project. In rhythm.</b><br />
  A calm terminal workspace for Claude Code, Codex and Cursor.
</p>

<p align="center">
  <a href="https://trungkien1208.github.io/orcha/">website</a> ·
  <a href="#try-it">try it</a> ·
  <a href="#what-you-get">what you get</a> ·
  <a href="#feedback">feedback</a>
</p>

<p align="center">
  <img src="docs/orcha/images/light.png" alt="orcha in the light theme: projects on the left, two agents side by side, fuel and git on the right" />
</p>

---

## What you get

| | |
|---|---|
| **Projects together** | Each project is a card in the sidebar with its agents under it. Open several at once. |
| **Live status** | See who is working, who is done, and who needs you. `PROCESS` shows each agent's stage: think → code → test → done. |
| **Fuel & accounts** | Plan limits (5h / week) for every Claude, Codex and Cursor account. Switch accounts in one click. All Claude chats move to the new account. |
| **Continue on…** | When the fuel runs low, orcha offers the best next place: another account, or hand the chat to another agent (whole conversation or a summary). |
| **Git in view** | Branch, changes and PR beside the work. One click opens your IDE or Git app. Ctrl+click opens any file path in an agent's output. |
| **Themes** | Orcha and Matcha, each in light and dark, plus the usual favorites. Font and ligatures from settings (Ghostty). |

<p align="center">
  <img src="docs/orcha/images/continue-on.png" alt="the CONTINUE ON popup: accounts with their fuel, the best one marked" width="80%" />
</p>

## Try it

macOS on Apple Silicon. Best in [Ghostty](https://ghostty.org); other terminals work with plain symbols instead of the agent logos.

One line, no questions:

```sh
curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh
```

It puts the app in `~/.local/bin/orcha` and the extras (fuel and account scripts, hand-off, theme, logo font) in `~/.orcha`. Run it again to update.

Then open a new terminal window where your projects live and run:

```sh
orcha
```

- `+ NEW` starts an agent.
- `⌘,` opens settings: theme, font, IDE, Git app.
- `ctrl+b q` detaches; run `orcha` again to come back. Your agents keep running.

Already use herdr? On first run orcha copies your herdr config and workspaces. herdr itself stays untouched.

## Feedback

This is an early build for friends to try. Something looks off or breaks? [Open an issue](https://github.com/trungkien1208/orcha/issues) with a screenshot. Ideas are welcome too.

## Build from source

You need Rust and [Zig 0.16](https://ziglang.org/download/).

```sh
git clone https://github.com/trungkien1208/orcha && cd orcha
ZIG=/path/to/zig cargo build --release
cp target/release/herdr ~/.local/bin/orcha   # the name "orcha" turns on the orcha app
```

## Thanks

orcha is a fork of [herdr](https://github.com/herdrdev/herdr). herdr does the hard part: the terminal runtime, sessions, and agent detection. orcha adds the look and the agent workflow on top. The original README is in [docs/orcha/herdr-README.md](docs/orcha/herdr-README.md).

Licensed under the [Apache License 2.0](LICENSE), like herdr.
