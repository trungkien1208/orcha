#!/usr/bin/env -S python3 -I
"""Your commits, grouped by task, across the repos you work in.

  worklog.py standup       "Yesterday / Today / Blockers", ready to paste
  worklog.py recap         today in numbers (commits, lines, tasks, quota, streak)
  worklog.py recap-short   one line for the evening notification

Repos: the git roots of the panes open in herdr, plus the folders in config
`worklog_dirs` (globs, e.g. "~/code/*"). Commits: yours (each repo's
user.email), all branches, no merges. Tasks: the task id in the branch that
holds the commit (or in its subject); titles from GitHub / Azure DevOps.
"""
import datetime as dt
import glob
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), "..", "lib"))
import orchalib as o  # noqa: E402

HERE = os.path.dirname(os.path.realpath(__file__))
sys.path.insert(0, HERE)
import tracker  # noqa: E402

CFG = o.load_config()
T = o.strings(CFG["lang"])
PREFIX = CFG["repo"]["branch_prefix"]


def run(argv, cwd=None):
    try:
        r = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=20)
        return r.stdout.strip() if r.returncode == 0 else ""
    except Exception:
        return ""


def repos():
    roots = set()
    try:
        panes = json.loads(run(["herdr", "pane", "list"]) or "{}").get("result", {}).get("panes", [])
    except ValueError:
        panes = []
    dirs = [p.get("cwd") for p in panes if p.get("cwd")]
    for pattern in CFG.get("worklog_dirs") or []:
        dirs += glob.glob(o.expand(pattern))
    for d in dirs:
        top = run(["git", "-C", d, "rev-parse", "--show-toplevel"])
        if top:
            roots.add(top)
    return sorted(roots)


def commits(repo, since, until=None):
    email = run(["git", "-C", repo, "config", "user.email"])
    if not email:
        return []
    args = ["git", "-C", repo, "log", "--all", "--no-merges", f"--author={email}", f"--since={since}",
            "--shortstat", "--format=@%H%x09%s%x09%aI"]
    if until:
        args.append(f"--until={until}")
    out, cur = [], None
    for line in run(args).splitlines():
        if line.startswith("@"):
            sha, subject, when = (line[1:].split("\t") + ["", ""])[:3]
            cur = {"sha": sha, "subject": subject, "when": when, "added": 0, "removed": 0, "repo": repo}
            out.append(cur)
        elif cur and "changed" in line:
            m = re.search(r"(\d+) insertion", line)
            cur["added"] = int(m.group(1)) if m else 0
            m = re.search(r"(\d+) deletion", line)
            cur["removed"] = int(m.group(1)) if m else 0
    return out


def is_feature(branch):
    return bool((PREFIX and PREFIX in branch) or re.search(r"feature|feat|fix|bug", branch))


def task_of(c):
    branches = run(["git", "-C", c["repo"], "branch", "-a", "--contains", c["sha"], "--format=%(refname:short)"]).splitlines()
    # prefer a feature branch over develop/release, which every merged commit is in
    branches.sort(key=lambda b: (not is_feature(b), len(b)))
    for b in branches:
        i = tracker.task_id(b.split("/", 1)[-1] if b.startswith("origin/") else b, CFG)
        if i:
            return i, b
    i = tracker.task_id(" " + c["subject"].replace(" ", "/") + "/", CFG)
    return i, (branches[0] if branches else "")


def grouped(cs):
    """[(repo name, id, title, state, [subjects])] in first-seen order."""
    groups, order = {}, []
    for c in cs:
        tid, branch = task_of(c)
        key = (c["repo"], tid or branch.split("/")[-1] or T["wl_other"])
        if key not in groups:
            groups[key] = {"id": tid, "branch": branch, "subjects": []}
            order.append(key)
        if c["subject"] not in groups[key]["subjects"]:
            groups[key]["subjects"].append(c["subject"])
    by_repo = {}
    for repo, _ in order:
        if repo not in by_repo:
            ids = [groups[k]["id"] for k in order if k[0] == repo and groups[k]["id"]]
            by_repo[repo] = tracker.get(ids, repo, CFG) if ids else {}
    result = []
    for key in order:
        g = groups[key]
        item = by_repo[key[0]].get(g["id"]) or {}
        name = item.get("title") or g["branch"].split("/")[-1].replace("-", " ") or key[1]
        result.append((os.path.basename(key[0]), g["id"], name, item.get("state", ""), g["subjects"]))
    return result


def previous_workday(today):
    d = today - dt.timedelta(days=1)
    while d.weekday() >= 5:
        d -= dt.timedelta(days=1)
    return d


def fmt_groups(groups, multi_repo):
    lines = []
    for repo, tid, name, state, subjects in groups:
        head = f"#{tid} {name}" if tid else name
        if state:
            head += f" ({state})"
        if multi_repo:
            head = f"[{repo}] {head}"
        lines.append(f"- {head}")
        for s in subjects[:4]:
            lines.append(f"    - {s}")
        if len(subjects) > 4:
            lines.append("    - " + T["wl_more_commits"].format(n=len(subjects) - 4))
    return lines


def standup():
    today = dt.date.today()
    prev = previous_workday(today)
    rs = repos()
    y = [c for r in rs for c in commits(r, f"{prev} 00:00", f"{today} 00:00")]
    t = [c for r in rs for c in commits(r, f"{today} 00:00")]
    gy, gt = grouped(y), grouped(t)
    multi = len({g[0] for g in gy + gt}) > 1
    days = T["weekdays"]
    label = T["wl_yesterday"] if (today - prev).days == 1 else f"{days[prev.weekday()]} ({prev:%d/%m})"
    out = [f"**{label}**"]
    out += fmt_groups(gy, multi) or [f"- ({T['wl_no_commits']})"]
    out += ["", f"**{T['wl_today']}**"]
    out += fmt_groups(gt, multi) or [f"- {T['wl_continue']}"]
    out += ["", f"**{T['wl_blockers']}**", f"- {T['wl_none']}"]
    return "\n".join(out)


def streak(rs):
    days = set()
    for r in rs:
        email = run(["git", "-C", r, "config", "user.email"])
        if email:
            days |= set(run(["git", "-C", r, "log", "--all", "--no-merges", f"--author={email}", "--since=90.days",
                             "--format=%ad", "--date=short"]).splitlines())
    d, n = dt.date.today(), 0
    if str(d) not in days:
        d -= dt.timedelta(days=1)
    while str(d) in days:
        n += 1
        d -= dt.timedelta(days=1)
    return n


def recap():
    today = dt.date.today()
    rs = repos()
    t = [c for r in rs for c in commits(r, f"{today} 00:00")]
    groups = grouped(t)
    added = sum(c["added"] for c in t)
    removed = sum(c["removed"] for c in t)
    quota = run([os.path.join(HERE, "limits.py"), "status"]) if CFG["accounts"] else ""
    s = streak(rs)
    out = [T["wl_recap_title"].format(day=T["weekdays"][today.weekday()], date=f"{today:%d/%m}"), ""]
    if t:
        out.append(T["wl_recap_line"].format(commits=len(t), added=added, removed=removed, tasks=len(groups)))
        out += [""] + fmt_groups(groups, len({g[0] for g in groups}) > 1)
    else:
        out.append(T["wl_recap_empty"])
    if s >= 2:
        out += ["", T["wl_streak"].format(n=s)]
    if quota:
        out += ["", quota]
    out += ["", T["wl_bye_friday"] if today.weekday() == 4 else T["wl_bye"]]
    return "\n".join(out), len(t), len(groups)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "standup"
    if cmd == "standup":
        print(standup())
    elif cmd == "recap":
        print(recap()[0])
    elif cmd == "recap-short":
        _, n, k = recap()
        print(T["wl_short"].format(commits=n, tasks=k) if n else T["wl_short_empty"])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
