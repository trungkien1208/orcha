#!/usr/bin/env -S python3 -I
"""Hand a pane's conversation to another agent (orcha's HAND OFF popup).

  handoff.py <pane> <agent> <mode>

agent: codex | claude | cursor.  mode: full (the whole conversation) | summary (the
new agent summarizes the notes first, then continues).

The old agent is not asked for anything, so this works when it is out of
fuel: the notes are built from its conversation file (Claude Code's
~/.claude*/projects/*/<session>.jsonl, Codex's ~/.codex/sessions/**), plus
the git state of the folder. They are written to ~/.cache/orcha/handoff/
and the new agent opens next to the old one, told to read them.
"""
import glob
import json
import os
import subprocess
import sys
import time

# the app that runs this pane (orcha or herdr), else whichever is on PATH
HERDR = os.environ.get("HERDR_BIN_PATH") or "herdr"

HOME = os.path.expanduser("~")
OUT_DIR = os.path.join(HOME, ".cache", "orcha", "handoff")
# keep the notes readable in one go
MAX_CHARS = {"full": 120_000, "summary": 60_000}


def herdr(*args):
    out = subprocess.run([HERDR, *args], capture_output=True, text=True, timeout=10).stdout
    return json.loads(out)["result"]


def text_of(content):
    """The plain text of a message's content (string or list of blocks)."""
    if isinstance(content, str):
        return content
    parts = []
    for block in content or []:
        if isinstance(block, dict) and block.get("type") in ("text", "input_text", "output_text"):
            parts.append(block.get("text", ""))
    return "\n".join(p for p in parts if p)


def claude_turns(session):
    paths = []
    for base in [os.path.join(HOME, ".claude")] + glob.glob(os.path.join(HOME, ".ai-accounts", "claude", "*")):
        paths += glob.glob(os.path.join(base, "projects", "*", f"{session}.jsonl"))
    if not paths:
        return []
    turns = []
    with open(max(paths, key=os.path.getmtime), errors="ignore") as fh:
        for line in fh:
            try:
                obj = json.loads(line)
            except ValueError:
                continue
            if obj.get("type") not in ("user", "assistant") or obj.get("isMeta"):
                continue
            msg = obj.get("message") or {}
            text = text_of(msg.get("content")).strip()
            # skip tool results and command noise
            if text and not text.startswith("<"):
                turns.append((msg.get("role") or obj["type"], text))
    return turns


def codex_turns(session):
    paths = glob.glob(os.path.join(HOME, ".codex", "sessions", "**", f"*{session}*.jsonl"), recursive=True)
    if not paths:
        return []
    turns = []
    with open(max(paths, key=os.path.getmtime), errors="ignore") as fh:
        for line in fh:
            try:
                obj = json.loads(line)
            except ValueError:
                continue
            payload = obj.get("payload") or {}
            if obj.get("type") == "response_item" and payload.get("type") == "message":
                text = text_of(payload.get("content")).strip()
                if text and not text.startswith("<"):
                    turns.append((payload.get("role", "?"), text))
    return turns


def git(cwd, *args):
    r = subprocess.run(["git", "-C", cwd, *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def notes(pane, target, mode):
    info = herdr("pane", "get", pane)["pane"]
    source = info.get("agent") or "agent"
    session = (info.get("agent_session") or {}).get("value")
    cwd = info.get("foreground_cwd") or info.get("cwd") or HOME
    tokens = info.get("tokens") or {}
    task = tokens.get("task") or info.get("terminal_title_stripped") or ""
    turns = []
    if session:
        turns = claude_turns(session) if source == "claude" else codex_turns(session)

    lines = [f"# Hand-off: {task or 'work in progress'}", ""]
    lines += [f"You are continuing work another coding agent ({source}) started in `{cwd}`.",
              "It stopped (usually out of plan limits), not because the work is done.", ""]
    if mode == "summary":
        lines += ["**First** summarize these notes in five bullets (goal, what is done, what is left, "
                  "open questions, next step). **Then** continue with the next step.", ""]
    else:
        lines += ["Read these notes, check the repository state yourself, then continue where it "
                  "stopped. Do not redo finished steps.", ""]
    branch = git(cwd, "branch", "--show-current")
    status = git(cwd, "status", "--short")
    stat = git(cwd, "diff", "HEAD", "--stat")
    if branch or status:
        lines += ["## Repository", "", f"- branch: `{branch or '(detached)'}`", ""]
        if status:
            lines += ["```", status[:4000], "```", ""]
        if stat:
            lines += ["```", stat[-4000:], "```", ""]
    if turns:
        first = next((t for r, t in turns if r == "user"), "")
        lines += ["## The goal (first request)", "", first[:6000], ""]
        lines += ["## Conversation (oldest first, text only)", ""]
        body = []
        for role, text in turns:
            who = "User" if role == "user" else source.capitalize()
            body.append(f"**{who}:** {text}\n")
        joined = "\n".join(body)
        limit = MAX_CHARS.get(mode, MAX_CHARS["full"])
        if len(joined) > limit:
            joined = "…(earlier turns cut)…\n\n" + joined[-limit:]
        lines.append(joined)
    else:
        lines += ["## Conversation", "", "(the old agent's conversation file was not found; "
                  "work from the repository state and the task above)", ""]
    return cwd, "\n".join(lines)


def main():
    if len(sys.argv) != 4 or sys.argv[2] not in ("codex", "claude", "cursor") or sys.argv[3] not in ("full", "summary"):
        sys.exit(__doc__)
    pane, target, mode = sys.argv[1:]
    cwd, text = notes(pane, target, mode)
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.join(OUT_DIR, f"{time.strftime('%Y%m%d-%H%M%S')}-{target}.md")
    with open(path, "w") as fh:
        fh.write(text)
    split = herdr("pane", "split", "--pane", pane, "--direction", "right", "--cwd", cwd)
    new_pane = split["pane"]["pane_id"]
    prompt = f"Read the hand-off notes in {path} and continue the work they describe."
    run = {"codex": "codex", "claude": "claude", "cursor": "cursor-agent"}[target]
    subprocess.run([HERDR, "pane", "run", new_pane, f"{run} {json.dumps(prompt)}"], capture_output=True)


if __name__ == "__main__":
    main()
