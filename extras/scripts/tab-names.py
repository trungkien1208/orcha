#!/usr/bin/env -S python3 -I
"""Name herdr tabs after what runs in them, instead of 1, 2, 3, and feed the
sidebar its tokens.

  <logo> <chat title>   an agent pane: the Claude / OpenAI (Codex) / Cursor logo
                        (font "Agent Icons", U+100000..U+100002); other agents a symbol (✦ Gemini...)
  ▶ expo start          a program running in a plain terminal
   my-app              an idle shell: its folder

Sidebar tokens: $count on each workspace ("· 2 agents"), and on each agent pane
$grp (its project's name, on the first agent of a project only) and $task
(its chat title).

Run every few seconds from the herdr tab bar (prints nothing). A tab you
renamed yourself is left alone: only tabs still named by their number, or by
this script's last name for them, are renamed.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), "..", "lib"))
import orchalib as o  # noqa: E402

CFG = o.load_config()
T = o.strings(CFG["lang"])
# one state file per herdr session (herdr --session ...): each names its own tabs
_SESSION = os.path.basename(os.path.dirname(os.environ.get("HERDR_SOCKET_PATH", ""))) or "default"
STATE = os.path.join(o.CACHE, f"tab-names-{_SESSION if _SESSION != 'herdr' else 'default'}.json")
MAX = 24
SHELLS = {"zsh", "bash", "sh", "fish", "-zsh", "login"}
LOGOS = CFG["widgets"]["logos"]
# logo glyph and display name per agent: the "Agent Icons" font has the real
# Claude / OpenAI / Cursor logos; the others (and all without the font) get a symbol
FONT_LOGO = {"claude": "\U00100000", "codex": "\U00100001", "cursor": "\U00100002"}
AGENT = {a: ((FONT_LOGO.get(a) if LOGOS else None) or icon, name) for a, (_, name, icon) in o.AGENTS.items()}
# A label that starts with one of these was written by this script. The saved
# copy can go missing; the mark is enough to keep owning it, so a Cursor pane
# does not stay stuck with the Codex logo.
OWNED_MARKS = set(FONT_LOGO.values()) | {icon for _, _, icon in o.AGENTS.values() if icon} | {"▶", "●"}
STATE_LABEL = {"working": T["st_working"], "blocked": T["st_blocked"], "done": T["st_done"], "idle": T["st_idle"]}
# processes that are background noise, never a tab's name
NOISE = {"caffeinate", "node", "python", "python3"}


def herdr(*args):
    r = subprocess.run(["herdr", *args], capture_output=True, text=True, timeout=10)
    return json.loads(r.stdout)["result"] if r.returncode == 0 and r.stdout else {}


def report(kind, target, tokens, *extra):
    args = []
    for k, v in tokens.items():
        args += ["--token", f"{k}={v}"] if v else ["--clear-token", k]
    subprocess.run(["herdr", kind, "report-metadata", target, "--source", "orcha", *args, *extra],
                   capture_output=True, timeout=10)


def clip(s, n=MAX):
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 1] + "…"


def program_name(pane_id):
    info = herdr("pane", "process-info", "--pane", pane_id).get("process_info", {})
    for p in info.get("foreground_processes", []):
        name = os.path.basename(p.get("name", ""))
        if name in SHELLS:
            continue
        argv = p.get("argv") or [name]
        # `node .../expo start` → "expo start"; `npm run dev` stays as is
        words = [os.path.basename(a) for a in argv if not a.startswith("-")]
        if name in NOISE and len(words) > 1:
            words = words[1:]
        if name in NOISE and len(words) <= 1:
            continue
        return " ".join(words[:3])
    return None


def label_for(pane):
    agent = pane.get("agent")
    title = (pane.get("terminal_title_stripped") or "").strip()
    if agent:
        icon = AGENT.get(agent, ("●", ""))[0]
        return f"{icon} {clip(title or agent, MAX - 2)}"
    prog = program_name(pane["pane_id"])
    # an app herdr does not detect as an agent often still sets a title
    # ("Cursor Agent"); a shell's own title is user@host or a path
    if title and "@" not in title and not title.startswith(("/", "~")) and title.split()[0] not in SHELLS:
        icon = AGENT["cursor"][0] if title.lower().startswith("cursor") else "●"
        return f"{icon} {clip(title, MAX - 2)}"
    if prog:
        return f"▶ {clip(prog, MAX - 2)}"
    folder = os.path.basename(pane.get("cwd") or "") or "~"
    return f" {clip(folder, MAX - 2)}"


def main():
    try:
        with open(STATE) as fh:
            last = json.load(fh)
    except (OSError, ValueError):
        last = {}

    tabs = herdr("tab", "list").get("tabs", [])
    panes = herdr("pane", "list").get("panes", [])
    agents = [p for p in panes if p.get("agent")]
    by_tab = {}
    for p in panes:
        by_tab.setdefault(p["tab_id"], []).append(p)

    # the name only. The pane border and the tab each draw the logo once;
    # putting it in the name as well showed two icons.
    for p in agents:
        _logo, name = AGENT.get(p["agent"], (None, None))
        want = name or None
        if want and p.get("display_agent") != want:
            labels = [a for st, txt in STATE_LABEL.items() for a in ("--state-label", f"{st}={txt}")]
            subprocess.run(["herdr", "pane", "report-metadata", p["pane_id"], "--source", "orcha",
                            "--display-agent", want, *labels], capture_output=True, timeout=10)

    # agents grouped by project: the first agent of each workspace carries the
    # project name ($grp, a header row); every agent carries its chat ($task).
    # Later agents of a project get a blank (U+2800) $grp: herdr indents every
    # row after the first, so this keeps them aligned under the project
    ws_label = {w["workspace_id"]: w.get("label", "") for w in herdr("workspace", "list").get("workspaces", [])}
    seen_ws = set()
    pane_sent = last.get("_panes", {})
    panes_now = {}
    for p in agents:
        ws = p["pane_id"].split(":")[0]
        grp = "⠀" if ws in seen_ws else ws_label.get(ws, "").strip()
        seen_ws.add(ws)
        tokens = {"grp": grp, "task": clip(p.get("terminal_title_stripped") or "", 30)}
        panes_now[p["pane_id"]] = tokens
        if pane_sent.get(p["pane_id"]) != tokens:
            report("pane", p["pane_id"], tokens)

    # projects list: how many agents run in each ($count)
    count_sent = last.get("_counts", {})
    counts_now = {}
    for ws in {t["workspace_id"] for t in tabs}:
        n = sum(1 for p in agents if p["pane_id"].split(":")[0] == ws)
        tokens = {"count": f"· {n} {T['agent'] if n == 1 else T['agents']}" if n else ""}
        counts_now[ws] = tokens
        if count_sent.get(ws) != tokens:
            report("workspace", ws, tokens)

    seen = {"_panes": panes_now, "_counts": counts_now}
    if CFG["widgets"]["tab_names"]:
        for tab in tabs:
            tid, label = tab["tab_id"], tab.get("label", "")
            previous = last.get(tid)
            is_ours = (
                label == str(tab.get("number"))
                or label == previous
                or (bool(label) and label[0] in OWNED_MARKS)
            )
            ps = by_tab.get(tid, [])
            if not is_ours or not ps:
                continue  # renamed by hand: leave it
            # the agent pane if there is one, else the focused one, else the first
            pane = next((p for p in ps if p.get("agent")), None) or next((p for p in ps if p.get("focused")), ps[0])
            want = label_for(pane)
            seen[tid] = want
            if want != label:
                subprocess.run(["herdr", "tab", "rename", tid, want], capture_output=True, timeout=10)

    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    with open(STATE, "w") as fh:
        json.dump(seen, fh)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        pass  # never break the tab bar
