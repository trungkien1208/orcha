# orcha — product spec

orcha is a terminal app for running many coding agents (Claude, Codex, Cursor,
…) across many projects. It is a fork of herdr (Apache-2.0, keep LICENSE and
NOTICE) that becomes its own app: run `orcha` in any terminal and get back to
your sessions.

This file is the source of truth for what to build. The visual reference is
the design canvas <https://claude.ai/artifact/M1NkNxt5eVigwctykcemRU>, boards
**FINAL · Calm light**, **FINAL · Calm dark**, **Popup · accounts** and
**Icon · matcha × whale** (option 2). When the build and this file disagree,
fix the build; when the mockup and this file disagree, ask.

Status legend: ✅ done · 🟡 partly · ⬜ not started.

---

## 1. Look: neo-brutal, calm

Thick black lines, flat bright fills, hard offset shadows, cream paper. Calm
means: **few things are raised, and color always means something.**

### Rules

- Hard shadow only on four things: the **NEW** button, the **selected project**
  card, the **PROCESS** card and the **main frame**. Everything else is flat.
- Color means agent or state, nothing else:
  - agent color: Claude yellow, Codex green, Cursor blue (train chip, pane title).
  - state: yellow working · green done · red needs you.
  - pink is the brand (ORCHA chip, task `#id` chip).
- Muted text for secondary facts (branch, counts, task names).

### Tokens

| token | Brutal light | Brutal dark |
|---|---|---|
| bg (page) | `#fff4d6` | `#1f1c19` |
| card | `#ffffff` | `#2e2a25` |
| frame (main frame fill) | `#f4ead0` | `#29261f` |
| ink (text) | `#111111` | `#f2e8d3` |
| line (borders) | `#111111` | `#0b0a09` |
| shadow | `#111111` | `#000000` |
| muted | `#6b6556` | `#b3a891` |
| pane / pane text | `#ffffff` / `#111111` | `#262320` / `#f2e8d3` |
| status bar bg / text | `#111111` / `#fff4d6` | `#0b0a09` / `#f2e8d3` |
| yellow | `#ffd23f` | `#f2c53d` |
| pink | `#ff7ab6` | `#f07aae` |
| green | `#7be0ad` | `#74d3a3` |
| blue | `#6c8cff` | `#7f9bff` |
| red | `#e5484d` | `#f26464` |
| rail done / rail ahead | `#111111` / `#c9c2ad` | `#9c927f` / `#4f493f` |

Dark mode rule: borders stay dark (black on charcoal), never bright.

### Terminal translation (cells, not pixels)

- Thick border (3px) → heavy box drawing `┏━┓┃┗┛`; thin border (2px) → light
  `┌─┐│└┘`; dashed (open, not selected) → `┄┆`.
- Hard shadow → half blocks in the shadow color: `▌` down the right edge,
  `▀` along the bottom, offset by one cell.
- Chips → a run of cells with a background color and bold text.
- Panes: white in light, so pane cells with the terminal's default colors are
  painted with pane / pane text.

### Styles

- Built in: **Brutal light** (default) and **Brutal dark**, following the
  terminal's light/dark appearance when `auto_switch` is on.
- Color variants keep the neo-brutal shapes and only swap tokens.
- Custom: a style file (`~/.config/orcha/styles/<name>.toml`) with the token
  table above.

### Popup template

Every popup follows the accounts popup (canvas board "Popup · accounts"):
search, settings, NEW picker, hand-off, confirmations, menus.

- Page behind is dimmed.
- Box: heavy border, bg fill, hard shadow (the only raised thing on screen).
- **Header** row on card fill, heavy line under it: bold UPPERCASE title,
  muted context ("this chat: ✳ Opus 5.5 · matcha-house"), `✕` outline button
  on the right.
- **Body**: rows with thin borders and gaps. The current / selected row gets
  the heavy border, card fill and yellow inner left edge (same as the selected
  project). Badges are small chips (`IN USE` ink, `MOST FUEL` green).
- One primary action per popup: raised green button. Other actions: outline
  buttons. Disabled/current: dashed outline, muted text.
- **Footer** row on card fill, heavy line above it: the keys (`↵`, `⌥↵`,
  letters) in bold with what they do, a muted extra action on the right.
- Keyboard first: arrows/`j k` move, `↵` primary, `esc` closes; every row is
  also clickable.
- A search popup puts the input as the first body row (heavy border while
  focused) and results below it in the same row style.

### Icon

A whale in a matcha bowl (canvas board "Icon", option 2): a black orca back
with a white eye patch rising out of a cream chawan, the matcha surface green,
three foam drops as the spout. Thick outline, flat fills.

- In the terminal: a glyph at U+100003 in the "Agent Icons" font that the
  setup installs; `◉` when the font is missing.
- SVG/PNG for README, app icon and website.

---

## 2. Layout

```
┌ header: [🍵🐋 ORCHA]  6 projects · 6 agents                    [⌘, settings] ┐
│ LEFT 280          │ MAIN                                       │ RIGHT 300   │
│ [+ NEW] [≡ menu]  │ ┏━ dark mode ● ━ explain theme ● ━ +1▾ + ┓ │ FUEL        │
│ PROJECTS · 6      │ ┃ ┌ ✳ OPUS 5.5 · dark mode ┐┌ ◎ GPT-6 … ┐┃ │ BRANCH      │
│ ┏ ▾ MATCHA-HOUSE ┓│ ┃ │                        ││           │┃ │ CHANGES     │
│ ┃ ■ Opus 5.5 …   ┃│ ┃ │        panes           ││           │┃ │ ▸ HISTORY   │
│ ┗━━━━━━━━━━━━━━━━┛│ ┃ └────────────────────────┘└───────────┘┃ │ ▸ WORKTREES │
│ ┆ ▾ ORCA-PAY     ┆│ ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛ │             │
│ ▸ TIDE-POOL  ■    │                                            │             │
│ ┏ PROCESS ━━━━━━┓ │                                            │             │
│ ┃ trains        ┃ │                                            │             │
├ status bar: [#42] Dark mode for the menu  ● open  3 files +26 −4   ◷ 18m 17:50 ┤
```

### Header (one row, full width) — 🟡

- Pink **ORCHA** chip with the icon.
- "N projects · M agents" in muted text.
- `⌘, settings` button on the right (outline, no shadow) opens settings.

### Left sidebar — 🟡

- Top: **+ NEW** (green, raised) and **≡ menu** (outline). Menu shows a dot
  when something needs attention.
- "PROJECTS · N" label.
- Each project is in one of three states:

| state | look | shows |
|---|---|---|
| selected (exactly one) | heavy border, white card, yellow inner left edge, hard shadow, bold name, `▾` | branch, every agent row |
| open (expanded, not selected) | dashed border, no fill, muted name, `▾` | branch, every agent row |
| folded | one row `▸ NAME` with a state dot per agent, or `idle` | — |

- Agent row: state dot, model name, task, red "needs you" when blocked. The
  agent in the open tab has a tinted row with an ink left bar, so sidebar,
  tab and pane point at the same thing.
- Click a name to select it; click `▸`/`▾` to open or fold without selecting.
- **PROCESS** card at the bottom (see §3.1).

### Main — 🟡

- One pane by default; tabs and splits are optional.
- Tabs belong to the selected project. They sit in the top edge of the main
  frame (no separate tab bar, no project chip: the sidebar already says which
  project is open).
- Tab = name + state dot. Open tab: ink background, bold. Overflow folds into
  `+N ▾`; `+` opens a new tab.
- Main frame: heavy border, frame fill, hard shadow.
- Pane: thin border, title bar in the agent's color with the model name and
  task (`✳ OPUS 5.5 · dark mode`).

### Right panel — 🟡

One thin-bordered box, sections split by lines, no shadows:

1. **FUEL** (when placed here): account email, then 5H and WEEK as ten
   segments + % left. Green > 50, yellow < 50, red < 20. Click → accounts
   popup (§3.2).
2. **BRANCH**: `⎇ name`, `↑2 to push`; buttons to open the IDE and Git GUI (§3.5).
3. **CHANGES**: `+26 −4`, files with colored letters M (amber) / A (green) / D (red).
4. **▸ HISTORY** folded by default: last commit and age. Open → commit graph
   drawn subway-style (lanes as colored rails, commits as stations, tags as
   chips), like Sourcetree.
5. **▸ WORKTREES** folded by default: count. Open → list, focused one bold,
   dashed `+ new worktree` button.

Hidden when the panes would drop below 60 columns.

### Status bar — 🟡

Black bar, full width, one row: pink `#42` chip, task title, `● open` in green
(PR state), muted `3 files +26 −4`, then on the right FUEL (when placed here),
`◷ 18m` elapsed, clock in bold.

FUEL placement option: right panel (default) or status bar, never both.

---

## 3. Features

### 3.1 PROCESS: agents as trains — 🟡

- Shows only agents that are running.
- Four stations: THINK → CODE → TEST → DONE. A train is a chip in the agent's
  color with its logo and `▶`, labelled by **model name** (OPUS 5.5,
  GPT-6 LUNA), never by agent name.
- Animated: the train chugs forward and back inside its current leg and shakes
  (a repaint tick only while trains are running).
- Station comes from the pane's `stage` token, else from agent status:
  - Claude: hook `orcha-stage.sh` (✅ in orcha-config): prompt / Read / Grep →
    thinking, Edit / Write → coding, test / typecheck / lint commands → testing.
  - Codex: same through its notify/hook support ⬜.
- Model name comes from a `model` token the hooks report ⬜, else the agent name.

### 3.2 Accounts and fuel — 🟡 (exists as an fzf picker in orcha-config)

- Accounts: Claude per config folder (company `~/.claude`, personal
  `~/.ai-accounts/claude/personal`), Codex, Cursor.
- Fuel per account: Claude from the usage endpoint with the account's
  Keychain sign-in (tokens on stdin only, never printed or logged), Codex from
  the rate limits in its newest session log, Cursor monthly usage.
- **Accounts popup** (canvas board "Popup · accounts"), opened by a click on
  FUEL or ⌘U:
  - header: ACCOUNTS and the chat it acts on.
  - one row per account: logo chip, name, email, 5H/WEEK gauges with reset time.
  - the current account: heavy border and yellow edge, `IN USE` badge.
  - the account with the most fuel: `MOST FUEL` badge, raised green `SWITCH ↵`.
  - not signed in: `SIGN IN`.
  - footer keys: `↵` switch this chat · `⌥↵` open in new tab · `H` hand off ·
    `+ add account`.

### 3.3 Switch account, same conversation — 🟡 (works in orcha-config)

- Hooks report `session_id` per pane.
- Switch = stop the agent, then `claude --resume <id>` with the other
  account's config folder (Codex: `codex resume <id>`). The conversation file
  is shared or copied first.
- **Low fuel**: under 10% orcha shows a toast "switch to personal (70% left)?";
  one click switches. When no account has fuel, it offers a hand-off (§3.4).

### 3.4 Hand off a conversation to another agent — ⬜

For when every account of one agent is out of fuel (Claude → Codex …).

- orcha builds a hand-off pack from the conversation file, without the old
  agent: the first goal, the last turns, files changed and git diff, test
  state, task `#id`.
- Opens the new agent in the same tab with the pack as its first prompt.
- Two modes: **full** (the whole history) and **summary** (the new agent
  summarizes first, then continues).

### 3.5 Open things — ⬜

- Cmd+click a path in a pane: file → editor (with `:line` when present),
  folder → Finder.
- BRANCH buttons: `⌘E` open the project in the IDE, `⌘G` open it in the Git GUI.
- Which IDE and Git GUI: detected on first use (VS Code, Cursor, Zed, JetBrains;
  Sourcetree, Fork, GitHub Desktop, lazygit), the user picks once.

### 3.6 Resume everything — ⬜

Quit or reboot, run `orcha`: every project, tab and pane comes back and each
agent continues its own conversation (`--resume` with the stored session id
and account).

### 3.7 Needs-you jump — ⬜

One key jumps to the next agent that needs you, across all projects, in
order of waiting time.

### 3.8 Train arrives at DONE — ⬜

When an agent finishes, its row shows the change summary (files, +/−) and
three actions: view diff, commit, open PR.

### 3.9 Smart NEW — ⬜

NEW opens a picker: project, agent, model, account (with fuel). Optional:
create a worktree and branch from a task id (`#42` → `feature/42-<slug>`).

### 3.10 Custom — ⬜

- Style files (§1).
- Keybindings.
- User commands in MENU ("run tests", "deploy staging"), global or per project.

### 3.11 Standalone app — ⬜

- Binary and command `orcha`; config in `~/.config/orcha/`, sessions in
  `~/.config/orcha/sessions/`.
- One-time import from herdr (config, sessions) on first run, with a notice.
- Works in any terminal; Ghostty is optional.

### Later

- Task link: read `#42` from the branch; show issue title and PR/CI state via `gh`.
- Stage timeline: how long each agent spent per station.

---

## 4. Build order

1. Brutal Calm UI: tokens, header with icon, sidebar three states, tabs in the
   frame, right panel box, status bar.
2. PROCESS with animated trains, `model` token, Codex stage.
3. Accounts popup + fuel (native), switch account, low-fuel toast.
4. Open things (Cmd+click, IDE, Git GUI).
5. Resume everything.
6. HISTORY graph, WORKTREES section.
7. Hand-off between agents.
8. Needs-you jump, DONE actions, smart NEW.
9. Standalone rename + herdr import.
10. Color variants and custom style files.

After each step: render the frame offline (`orcha_preview` test →
`render_frame.py`) and compare with the mockup before calling it done.

## 5. Dev notes

- Rust 1.96.1 (`/opt/homebrew/opt/rustup/bin`), Zig 0.16.0 (`ZIG` env). Zig's
  own TLS fails on macOS here: fetch zig deps with curl, then `zig fetch <file>`.
- Run tests by module or single-threaded; the full parallel suite flakes on HOME.
- Dev session: `herdr --session orcha-dev` in a clean env; CLI talks to it with
  `HERDR_SOCKET_PATH=~/.config/herdr/sessions/orcha-dev/herdr.sock`.
- `ORCHA_LAYOUT_DEFAULT = !cfg!(test)` keeps upstream tests on herdr's layout.
- Personal GitHub `trungkien1208`, commit email `kodaigailaem1@gmail.com`; no
  personal project names in demos or screenshots.
