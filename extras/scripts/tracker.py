#!/usr/bin/env -S python3 -I
"""Task tracker lookups for the repo a pane is in: GitHub Issues or Azure DevOps,
picked from the repo's origin URL. Results are cached for 30 minutes.

  tracker.py id <branch>                  the task id in a branch name, or nothing
  tracker.py get <id> [<id>...] [--repo DIR]
        JSON {id: {title, state, type, summary, assignee, url}}; ids it could not
        read are left out
  tracker.py pr <branch> <target> [--repo DIR]
        the web page that creates a pull request from branch into target

GitHub uses the `gh` CLI's own sign-in. Azure DevOps reads a personal access
token from $AZURE_DEVOPS_PAT, else from the file named in config
tracker.azure_pat_file (an AZURE_DEVOPS_PAT=... line). The token is passed to
curl on stdin, never on its command line.
"""
import base64
import html
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), "..", "lib"))
import orchalib as o  # noqa: E402

TTL = 30 * 60


def run(argv, cwd=None, stdin=None, timeout=20):
    try:
        r = subprocess.run(argv, cwd=cwd, input=stdin, capture_output=True, text=True, timeout=timeout)
        return r.stdout.strip() if r.returncode == 0 else ""
    except Exception:
        return ""


def task_id(branch, cfg=None):
    cfg = cfg or o.load_config()
    m = re.search(cfg["tracker"]["task_pattern"], branch or "")
    return m.group(1) if m else None


def remote_of(repo):
    """('github', 'owner/repo') | ('azure', (org, project, repo)) | (None, None)"""
    url = run(["git", "-C", repo, "remote", "get-url", "origin"])
    m = re.search(r"github\.com[:/]([^/]+)/([^/]+?)(?:\.git)?/?$", url)
    if m:
        return "github", f"{m.group(1)}/{m.group(2)}"
    m = (re.search(r"ssh\.dev\.azure\.com:v3/([^/]+)/([^/]+)/([^/]+?)(?:\.git)?$", url)
         or re.search(r"dev\.azure\.com/([^/]+)/([^/]+)/_git/([^/]+?)(?:\.git)?$", url))
    if m:
        return "azure", tuple(urllib.parse.unquote(x) for x in m.groups())
    m = re.search(r"([^/.@]+)\.visualstudio\.com/(?:DefaultCollection/)?([^/]+)/_git/([^/]+?)(?:\.git)?$", url)
    if m:
        return "azure", tuple(urllib.parse.unquote(x) for x in m.groups())
    return None, None


def clean(text, n=400):
    text = re.sub(r"<br\s*/?>|</(p|div|li|h\d)>", " ", text or "", flags=re.I)
    text = html.unescape(re.sub(r"<[^>]+>", "", text))
    text = re.sub(r"[#*_`>]+", " ", text)  # markdown marks in GitHub bodies
    text = " ".join(text.split())
    return text if len(text) <= n else text[: n - 1] + "…"


def azure_pat(repo, cfg):
    if os.environ.get("AZURE_DEVOPS_PAT"):
        return os.environ["AZURE_DEVOPS_PAT"].strip()
    path = o.expand((cfg["tracker"].get("azure_pat_file") or "").replace("{repo}", repo))
    try:
        with open(path) as fh:
            m = re.search(r"^\s*(?:export\s+)?AZURE_DEVOPS_PAT\s*=\s*['\"]?([^'\"\s]+)", fh.read(), re.M)
        return m.group(1) if m else None
    except OSError:
        return None


def fetch_azure(ids, where, repo, cfg):
    org, project, _ = where
    pat = azure_pat(repo, cfg)
    if not pat:
        return {}
    auth = base64.b64encode(f":{pat}".encode()).decode()
    body = json.dumps({"ids": [int(i) for i in ids], "errorPolicy": "omit",
                       "fields": ["System.Title", "System.State", "System.WorkItemType",
                                  "System.AssignedTo", "System.Description"]})
    base = f"https://dev.azure.com/{urllib.parse.quote(org)}/{urllib.parse.quote(project)}"
    out = run(["curl", "-sS", "-f", "-m", "15", "-K", "-", "-H", "Content-Type: application/json",
               "--data-raw", body, f"{base}/_apis/wit/workitemsbatch?api-version=7.1"],
              stdin=f'header = "Authorization: Basic {auth}"\n')
    found = {}
    try:
        for v in json.loads(out).get("value") or []:
            if not v:
                continue
            f = v["fields"]
            found[str(v["id"])] = {
                "title": f.get("System.Title", ""), "state": f.get("System.State", ""),
                "type": f.get("System.WorkItemType", ""),
                "assignee": (f.get("System.AssignedTo") or {}).get("displayName"),
                "summary": clean(f.get("System.Description", "")),
                "url": f"{base}/_workitems/edit/{v['id']}",
            }
    except (ValueError, KeyError, TypeError):
        pass
    return found


def fetch_github(ids, where):
    found = {}
    for i in ids:
        out = run(["gh", "issue", "view", str(i), "--repo", where,
                   "--json", "number,title,state,body,assignees,url,labels"])
        try:
            d = json.loads(out)
        except ValueError:
            continue
        found[str(i)] = {
            "title": d.get("title", ""), "state": (d.get("state") or "").capitalize(),
            "type": next((lb["name"] for lb in d.get("labels") or [] if lb["name"] in ("bug", "feature", "enhancement")), "Issue"),
            "assignee": ", ".join(a.get("login", "") for a in d.get("assignees") or []) or None,
            "summary": clean(d.get("body", "")), "url": d.get("url", ""),
        }
    return found


def get(ids, repo, cfg=None):
    cfg = cfg or o.load_config()
    kind, where = remote_of(repo)
    if not kind:
        return {}
    key = re.sub(r"[^A-Za-z0-9]+", "_", f"{kind}-{where if isinstance(where, str) else '-'.join(where[:2])}")
    cache = os.path.join(o.CACHE, "tracker")
    os.makedirs(cache, exist_ok=True)
    found, missing = {}, []
    for i in dict.fromkeys(str(x) for x in ids):
        path = os.path.join(cache, f"{key}-{i}.json")
        if os.path.exists(path) and time.time() - os.path.getmtime(path) < TTL:
            try:
                with open(path) as fh:
                    found[i] = json.load(fh)
                continue
            except (OSError, ValueError):
                pass
        missing.append(i)
    if missing:
        fresh = fetch_azure(missing, where, repo, cfg) if kind == "azure" else fetch_github(missing, where)
        for i, item in fresh.items():
            item["id"] = i
            with open(os.path.join(cache, f"{key}-{i}.json"), "w") as fh:
                json.dump(item, fh)
            found[i] = item
    return found


def pr_url(branch, target, repo):
    kind, where = remote_of(repo)
    q = urllib.parse.quote
    if kind == "github":
        return f"https://github.com/{where}/compare/{q(target, safe='')}...{q(branch, safe='')}?expand=1"
    if kind == "azure":
        org, project, name = (q(x, safe="") for x in where)
        return (f"https://dev.azure.com/{org}/{project}/_git/{name}/pullrequestcreate"
                f"?sourceRef={q(branch, safe='')}&targetRef={q(target, safe='')}")
    return None


def main():
    args = sys.argv[1:]
    repo = os.getcwd()
    if "--repo" in args:
        i = args.index("--repo")
        repo = args[i + 1]
        del args[i:i + 2]
    top = run(["git", "-C", repo, "rev-parse", "--show-toplevel"]) or repo
    if args[:1] == ["id"] and len(args) == 2:
        print(task_id(args[1]) or "")
    elif args[:1] == ["get"] and len(args) > 1:
        print(json.dumps(get(args[1:], top), ensure_ascii=False))
    elif args[:1] == ["pr"] and len(args) == 3:
        print(pr_url(args[1], args[2], top) or "")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
