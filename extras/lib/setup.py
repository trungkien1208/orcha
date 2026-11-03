"""`orcha setup`: as few questions as it can, then `orcha apply`.

Quick (3-4 questions): language, quick or custom, colors, git app. Everything
else is found on this machine: the agents installed, the Claude accounts and
their emails, the editor, GitHub sign-in, missing tools.
Custom adds: editor, plan usage on / off, Azure DevOps, widget bundle.
Run it again any time; the first choice in each menu is what you have now.
"""
import getpass
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import orchalib as o  # noqa: E402

T = {}
ME = os.path.realpath(__file__)
TOOLS = [("fzf", "fzf"), ("fd", "fd"), ("rg", "ripgrep"), ("bat", "bat"), ("jq", "jq"), ("yazi", "yazi"),
         ("herdr", "herdr"), ("lame", "lame")]
JETBRAINS = ("IntelliJ IDEA", "IntelliJ IDEA CE", "IntelliJ IDEA Ultimate", "WebStorm", "PyCharm", "PyCharm CE",
             "GoLand", "PhpStorm", "RubyMine", "Rider", "CLion", "RustRover")
FREE_MAIL = ("gmail.", "googlemail.", "outlook.", "hotmail.", "live.", "icloud.", "me.com", "yahoo.", "proton")


# ---------- menus

def fzf_opts():
    p = o.palette()
    return (f"--border=rounded --info=hidden --pointer='▌' --no-scrollbar "
            f"--color=bg+:{p['active_row_bg']},fg:{p['subtext0']},fg+:{p['fg']},hl:{p['accent']},hl+:{p['red']},"
            f"prompt:{p['accent']},pointer:{p['red']},border:{p['dim']},header:{p['blue']}")


def menu(question, rows, hint="", preview=None, esc="first"):
    """rows: [(id, label)], the first one picked by Enter; returns the id. On
    Esc: the first row's id (so Esc never breaks setup), or `esc` itself."""
    env = dict(os.environ, FZF_DEFAULT_OPTS=fzf_opts())
    args = ["fzf", "--ansi", "--delimiter=\t", "--with-nth=2", "--no-sort", "--cycle", "--no-input",
            "--layout=reverse", f"--height={min(len(rows) + 6, 24) if not preview else 24}",
            f"--border-label= {question} ", "--border-label-pos=3"]
    if hint:
        args.append(f"--header={hint}")
    if preview:
        args += [f"--preview={preview}", "--preview-window=right,55%,border-left,wrap"]
    r = subprocess.run(args, input="".join(f"{i}\t{label}\n" for i, label in rows),
                       capture_output=True, text=True, env=env)
    out = r.stdout.strip()
    if not out:
        return rows[0][0] if esc == "first" else esc
    return out.split("\t", 1)[0]


def yes_no(question, default=True, hint=""):
    rows = [("yes", "✓  " + T["s_yes"]), ("no", "✗  " + T["s_no"])]
    return menu(question, rows if default else rows[::-1], hint) == "yes"


def mark_current(rows, cur):
    """The current choice first and tagged, so Enter keeps it."""
    muted = o.palette()["muted"]
    rows = [(i, f"{label}  {o.fg(muted, '· ' + T['s_current'])}" if i == cur else label) for i, label in rows]
    return sorted(rows, key=lambda r: r[0] != cur)


def say(text, color="accent", bold=False):
    print("  " + o.fg(o.palette()[color], text, bold=bold))


# ---------- what is on this machine

def app_installed():
    """The orcha app (a real binary at ~/.local/bin/orcha, not our link)."""
    path = os.path.join(o.HOME, ".local", "bin", "orcha")
    return os.path.isfile(path) and not os.path.islink(path)


def missing_tools():
    # the orcha app replaces herdr
    skip = {"herdr"} if app_installed() else set()
    return [brew for cmd, brew in TOOLS if cmd not in skip and not shutil.which(cmd)]


def ensure_tools():
    """Before any menu: the menus are fzf, so a missing fzf is asked about in
    plain text. One question, Enter installs everything missing."""
    missing = missing_tools()
    if not missing:
        return True
    if not shutil.which("brew"):
        print(f"\n  {T['s_need_brew']}\n")
        return False
    print(f"\n  🐋🍵 {T['s_install_tools'].format(tools=', '.join(missing))}")
    try:
        answer = input(f"  [{T['s_yes']}/{T['s_no'].lower()}] (Enter = {T['s_yes']}): ").strip().lower()
    except EOFError:
        answer = ""
    if answer and not answer.startswith(("y", "c")):  # yes / có
        return shutil.which("fzf") is not None
    subprocess.run(["brew", "install", *missing])
    return shutil.which("fzf") is not None


def ask_ghostty():
    """Ghostty is optional: herdr runs in any terminal. Without it there are no
    Cmd shortcuts (Ctrl+B + key instead), no orcha theme / font / cursor trail."""
    if o.app_installed("Ghostty") or not shutil.which("brew"):
        return
    rows = [("install", "👻  " + T["s_ghostty_install"]), ("keep", "🖥   " + T["s_ghostty_keep"])]
    if menu(T["s_ghostty"], rows, T["s_ghostty_hint"]) == "install":
        subprocess.run(["brew", "install", "--cask", "ghostty"])


def installed_agents():
    return o.installed_agents()


def claude_email(folder):
    return o.claude_email(folder)


def name_for(email, taken):
    if not email:
        name = "main"
    elif any(f in email.split("@")[-1] for f in FREE_MAIL):
        name = "personal"
    else:
        name = "work"
    n, i = name, 2
    while n in taken:
        n, i = f"{name}{i}", i + 1
    return n


def find_accounts(cfg, agents):
    """Claude: ~/.claude plus each account folder already set up (in config, or
    in ~/.ai-accounts/claude/*), named by their email. Codex / Cursor: one each."""
    accounts, taken = [], set()
    known = {o.expand(a["dir"]): a for a in cfg["accounts"] if a["kind"] == "claude" and a.get("dir")}
    # account folders other tools / people make: ~/.ai-accounts/claude/<name>, ~/.claude-<name>
    root = os.path.join(o.HOME, ".ai-accounts", "claude")
    for d in sorted(os.listdir(root)) if os.path.isdir(root) else []:
        known.setdefault(os.path.join(root, d), {"id": f"claude-{d}"})
    for d in sorted(os.listdir(o.HOME)):
        folder = os.path.join(o.HOME, d)
        if d.startswith(".claude-") and os.path.exists(os.path.join(folder, ".claude.json")):
            known.setdefault(folder, {"id": "claude-" + d.removeprefix(".claude-")})
    if "claude" in agents:
        email = claude_email(None)
        old = next((a for a in cfg["accounts"] if a["kind"] == "claude" and not a.get("dir")), None)
        name = old["id"].removeprefix("claude-") if old else name_for(email, taken)
        taken.add(name)
        accounts.append({"id": f"claude-{name}", "kind": "claude", "label": f"Claude · {name}", "email": email, "dir": None})
        for folder, old in known.items():
            if not os.path.isdir(folder):
                continue
            name = old["id"].removeprefix("claude-")
            base, i = name, 2
            while name in taken:  # ~/.claude got the same name from its email
                name, i = f"{base}{i}", i + 1
            taken.add(name)
            accounts.append({"id": f"claude-{name}", "kind": "claude", "label": f"Claude · {name}",
                             "email": claude_email(folder), "dir": folder.replace(o.HOME, "~", 1)})
    for kind in ("codex", "cursor"):
        if kind in agents:
            accounts.append({"id": kind, "kind": kind, "label": o.AGENTS[kind][1], "email": "", "dir": None})
    return accounts


def ask_account_names(cfg):
    """With 2+ Claude accounts, a name per account (for `oc <name>`)."""
    claude = [a for a in cfg["accounts"] if a["kind"] == "claude"]
    if len(claude) < 2:
        return
    taken = set()
    for a in claude:
        pick_name(a, taken)
        taken.add(a["id"].removeprefix("claude-"))


def pick_name(a, taken, esc="first"):
    """A name for one Claude account, picked from suggestions made of its
    email; typing only for "other". Returns False when left with Esc."""
    cur = a["id"].removeprefix("claude-")
    email = a.get("email") or ""
    local, _, domain = email.partition("@")
    org = domain.split(".")[0] if domain else ""
    is_free = any(f in domain for f in FREE_MAIL)
    ideas = [cur, "personal" if is_free else "work", "company" if not is_free else "me",
             org if org and not is_free else "", re.sub(r"[^a-z0-9]+", "", local.lower())]
    ideas = [n for n in dict.fromkeys(ideas) if n and (n == cur or n not in taken)]
    rows = [(n, f"oc {n}") for n in ideas] + [("✎", "✎  " + T["s_name_other"])]
    who = email or (a.get("dir") or "~/.claude")
    pick = menu(T["s_name_account"].format(who=who), mark_current(rows, cur), T["s_name_hint"], esc=esc)
    if pick == esc and esc != "first":
        return False
    if pick == "✎":
        typed = input(f"  {T['s_name_type']}: ").strip().lower()
        pick = re.sub(r"[^a-z0-9_-]+", "-", typed) or cur
        if pick in taken and pick != cur:
            pick = cur
    a["id"], a["label"] = f"claude-{pick}", f"Claude · {pick}"
    return True


def find_editors():
    out = []
    for k, (_, app) in o.IDES.items():
        if k == "jetbrains":
            app = next((a for a in JETBRAINS if o.app_installed(a)), "")
            if app:
                out.append((k, app))
        elif (app and o.app_installed(app)) or (k in o.IDE_CLI and shutil.which(o.IDE_CLI[k])):
            out.append((k, app))
    return out


def find_git_apps():
    return [k for k, (_, app) in o.GIT_APPS.items()
            if (app and o.app_installed(app)) or (k in o.GIT_CLI and shutil.which(o.GIT_CLI[k]))]


# ---------- preview of a palette (fzf preview)

def preview_palette(name):
    cfg = o.load_config()
    cfg["palette"], cfg["accent"] = name, ""
    p = o.palette(cfg)
    if p.get("native"):
        global T
        T = o.strings()
        print("\n  " + T["p_default"] + "\n\n  " + T["p_default_hint"])
        return

    def bg(c, s):
        r, g, b = o.rgb(c)
        return f"\033[48;2;{r};{g};{b}m{s}\033[0m"
    on_bg = lambda c, s: f"\033[38;2;{';'.join(map(str, o.rgb(c)))}m{s}\033[39m"  # noqa: E731
    pad = lambda s, n=38: s + " " * max(0, n - len(re.sub(r"\033\[[0-9;]*m", "", s)))  # noqa: E731
    line = lambda s: print("  " + bg(p["bg"], pad(s)))  # noqa: E731
    print(o.fg(p["accent"], f"  {p['name']}", bold=True), o.fg(p["muted"], f" {p['description']}"))
    print()
    print("  " + "".join(bg(c, "   ") for c in p["ansi"][:8]))
    print("  " + "".join(bg(c, "   ") for c in p["ansi"][8:]))
    print()
    line("")
    line(" " + on_bg(p["accent"], "~/my-app") + on_bg(p["muted"], " on ") + on_bg(p["magenta"], "⎇ main"))
    line(" " + on_bg(p["fg"], "$ git status"))
    line(" " + on_bg(p["green"], "  + src/new.ts"))
    line(" " + on_bg(p["yellow"], "  ● src/app.ts"))
    line(" " + on_bg(p["red"], "  ✗ old.js"))
    line(" " + on_bg(p["peach"], "  ✳ Claude") + on_bg(p["muted"], " · working"))
    line("")


def preview_bundle(name):
    p = o.palette()
    on = set(o.BUNDLES.get(name, []))
    for k in o.WIDGETS:
        if name == "custom":
            print("  " + o.fg(p["muted"], "· " + T["w_" + k]))
        else:
            print("  " + (o.fg(p["green"], "✓ " + T["w_" + k]) if k in on else o.fg(p["dim"], "· " + T["w_" + k])))


# ---------- questions

def ask_lang(cfg):
    names = {"en": "English", "vi": "Tiếng Việt"}
    cfg["lang"] = menu("Language · Ngôn ngữ", mark_current([(k, names.get(k, k)) for k in o.languages()], cfg["lang"]))


def ask_palette(cfg):
    rows = []
    for name in o.palette_names():
        p = o.palette(dict(cfg, palette=name, accent=""))
        if p.get("native"):
            rows.append((name, f"○○○○○○  {T['p_default'].split(':')[0]}"))
            continue
        swatch = "".join(o.fg(p[k], "●") for k in ("accent", "red", "green", "yellow", "blue", "magenta"))
        rows.append((name, f"{swatch}  {p['name']}"))
    cfg["palette"] = menu(T["s_palette"], mark_current(rows, cfg["palette"]), T["s_palette_hint"],
                          preview=f"{sys.executable} -I {ME} preview {{1}}")


def ask_git_app(cfg):
    found = find_git_apps()
    if not found:
        cfg["git_app"] = "none"
        return
    rows = [(k, o.GIT_APPS[k][0]) for k in found] + [("none", T["s_git_none"])]
    cur = cfg["git_app"] if cfg["git_app"] in found + ["none"] else found[0]
    cfg["git_app"] = menu(T["s_git_app"], mark_current(rows, cur), T["s_git_app_hint"])


def ask_editor(cfg, editors):
    if len(editors) <= 1:
        return
    rows = [(k, app if k == "jetbrains" else o.IDES[k][0]) for k, app in editors]
    cfg["ide"] = menu(T["s_ide"], mark_current(rows, cfg["ide"]), T["s_ide_hint"])
    cfg["ide_app"] = dict(editors).get(cfg["ide"], "")


def ask_azure(cfg):
    path = o.expand(cfg["tracker"]["azure_pat_file"])
    has = os.path.exists(path)
    if not yes_no(T["s_azure"], default=has, hint=T["s_azure_hint"]):
        return
    say(T["s_azure_pat_hint"], "muted")
    pat = getpass.getpass(f"  {T['s_azure_pat']}{' (' + T['s_keep'] + ')' if has else ''}: ").strip()
    if pat:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as fh:
            fh.write(f"AZURE_DEVOPS_PAT={pat}\n")
        say("✓ " + T["s_saved_to"].format(path=path.replace(o.HOME, "~")), "green")


def ask_widgets(cfg):
    w = cfg["widgets"]
    cur = next((b for b, on in o.BUNDLES.items() if set(on) == {k for k in o.WIDGETS if w[k]}), "custom")
    rows = [("focus", "🎯  " + T["b_focus"]), ("full", "✨  " + T["b_full"]),
            ("minimal", "🍃  " + T["b_minimal"]), ("custom", "🛠  " + T["b_custom"])]
    pick = menu(T["s_widgets"], mark_current(rows, cur), preview=f"{sys.executable} -I {ME} bundle {{1}}")
    if pick != "custom":
        usage = w["limits"]
        for k in o.WIDGETS:
            w[k] = k in o.BUNDLES[pick]
        w["limits"] = w["limits"] and usage  # the usage answer wins over the bundle
        return
    while True:
        rows = [("done", o.fg(o.palette()["green"], "✓ " + T["s_widgets_done"], bold=True))]
        rows += [(k, f"{'◉' if w[k] else '○'}  {T['w_' + k]}") for k in o.WIDGETS]
        pick = menu(T["s_widgets"], rows, T["s_widgets_hint"])
        if pick == "done":
            return
        w[pick] = not w[pick]


# ---------- doing it

def bootstrap():
    """`orcha` on PATH and the zsh line (what an installer would do)."""
    notes = []
    bin_dir = os.path.join(o.HOME, ".local", "bin")
    link = os.path.join(bin_dir, "orcha")
    target = os.path.join(o.ORCHA_HOME, "bin", "orcha")
    # never replace the orcha app: it hands `orcha setup|menu|…` to us itself
    if not app_installed() and os.path.realpath(link) != target:
        os.makedirs(bin_dir, exist_ok=True)
        if os.path.lexists(link):
            os.remove(link)
        os.symlink(target, link)
        notes.append(T["s_note_linked"])
    zshrc = os.path.join(o.HOME, ".zshrc")
    try:
        text = open(zshrc).read()
    except OSError:
        text = ""
    add = []
    if "shell/orcha.zsh" not in text:
        add.append(f'[ -f "{o.ORCHA_HOME}/shell/orcha.zsh" ] && source "{o.ORCHA_HOME}/shell/orcha.zsh"  # orcha')
    if bin_dir not in os.environ.get("PATH", "") and ".local/bin" not in text:
        add.append('export PATH="$HOME/.local/bin:$PATH"  # orcha')
    if add:
        with open(zshrc, "a") as fh:
            fh.write("\n" + "\n".join(add) + "\n")
        notes.append(T["s_note_zshrc"])
    return notes


def install_extras(cfg):
    notes = []
    fonts = os.path.join(o.HOME, "Library", "Fonts")
    have = " ".join(os.listdir(fonts)).lower() if os.path.isdir(fonts) else ""
    cask = o.FONTS.get(cfg["font"]["family"])
    if cask and cfg["font"]["family"].split()[0].lower() not in have and shutil.which("brew"):
        subprocess.run(["brew", "install", "--cask", cask], capture_output=True)
        notes.append(T["s_note_font"].format(font=cfg["font"]["family"]))
    if cfg["widgets"]["logos"] and not os.path.exists(os.path.join(fonts, "AgentIcons.ttf")):
        r = subprocess.run([os.path.join(o.ORCHA_HOME, "fonts", "build.sh")], capture_output=True, text=True)
        notes.append(T["s_note_logos"] if r.returncode == 0 else T["s_note_logos_failed"])
    if shutil.which("claude"):
        listed = subprocess.run(["claude", "plugin", "list"], capture_output=True, text=True).stdout
        if "orcha@orcha" not in listed:
            subprocess.run(["claude", "plugin", "marketplace", "add", o.ORCHA_HOME], capture_output=True)
            r = subprocess.run(["claude", "plugin", "install", "orcha@orcha", "--scope", "user"], capture_output=True)
            notes.append(T["s_note_plugin"] if r.returncode == 0 else T["s_note_plugin_failed"])
    return notes


def summary(cfg, agents, notes):
    p = o.palette(cfg)
    print()
    say(T["s_finished"], bold=True)
    print()
    names = ", ".join(o.AGENTS[a][1] for a in agents) or "—"
    claude = [f"{a['id'].removeprefix('claude-')} ({a['email'] or T['s_not_signed_in']})"
              for a in cfg["accounts"] if a["kind"] == "claude"]
    ide = cfg.get("ide_app") or o.IDES[cfg["ide"]][0]
    on = [T["w_" + k].split(":")[-1].strip() for k in o.WIDGETS if cfg["widgets"][k]]
    rows = [
        (T["s_sum_colors"], p["name"]),
        (T["s_sum_agents"], names),
        (T["s_sum_claude"], ", ".join(claude) or "—"),
        (T["s_sum_editor"], f"{ide} · git: {o.GIT_APPS[cfg['git_app']][0]}"),
        (T["s_sum_usage"], T["s_yes"] if cfg["widgets"]["limits"] else T["s_no"]),
        (T["s_sum_widgets"], f"{len(on)}/{len(o.WIDGETS)}"),
    ]
    for k, v in rows:
        print(f"  {o.fg(p['muted'], f'{k:<12}')} {v}")
    for n in notes:
        print("  " + o.fg(p["muted"], "· " + n))
    print()
    say(T["s_finished_hint"], "fg")
    say(T["s_change_later"], "muted")


def app_setup():
    """`setup --app`, run by the orcha app's installer: no questions. Finds
    the agents, accounts, editor and git app, picks the defaults, applies."""
    global T
    cfg = o.load_config()
    T = o.strings(cfg["lang"])
    agents = installed_agents()
    cfg["accounts"] = find_accounts(cfg, agents)
    editors = find_editors()
    if cfg["ide"] not in dict(editors) and editors:
        cfg["ide"], cfg["ide_app"] = editors[0]
    git_apps = find_git_apps()
    if cfg["git_app"] not in git_apps:
        cfg["git_app"] = git_apps[0] if git_apps else "none"
    o.save_config(cfg)
    # the stage hook reads JSON with jq
    if not shutil.which("jq") and shutil.which("brew"):
        subprocess.run(["brew", "install", "jq"], capture_output=True)
    import apply
    apply.apply(quiet=True)
    notes = bootstrap() + install_extras(cfg)
    names = ", ".join(o.AGENTS[a][1] for a in agents) or "none found yet"
    print(f"  agents: {names}")
    for n in notes:
        print("  · " + n)


def main():
    global T
    cfg = o.load_config()
    T = o.strings(cfg["lang"])
    if not ensure_tools():
        sys.exit(1)
    ask_lang(cfg)
    T = o.strings(cfg["lang"])

    ask_ghostty()

    is_custom = menu(T["s_mode"], [("quick", "⚡  " + T["s_quick"]), ("custom", "🎨  " + T["s_custom"])]) == "custom"
    ask_palette(cfg)
    o.save_config(cfg)  # the next menus are drawn in the new colors

    agents = installed_agents()
    cfg["accounts"] = find_accounts(cfg, agents)
    ask_account_names(cfg)
    editors = find_editors()
    if cfg["ide"] not in dict(editors) and editors:
        cfg["ide"], cfg["ide_app"] = editors[0]
    ask_git_app(cfg)
    if is_custom:
        ask_editor(cfg, editors)
        if any(a in o.LIMIT_KINDS for a in agents):
            cfg["widgets"]["limits"] = yes_no(T["s_usage"], cfg["widgets"]["limits"], T["s_usage_hint"])
        ask_azure(cfg)
        ask_widgets(cfg)
    o.save_config(cfg)

    import apply
    apply.apply(quiet=True)
    notes = bootstrap() + install_extras(cfg)
    summary(cfg, agents, notes)


if __name__ == "__main__":
    if sys.argv[1:2] == ["preview"]:
        preview_palette(sys.argv[2])
    elif sys.argv[1:2] == ["--app"]:
        app_setup()
    elif sys.argv[1:2] == ["bundle"]:
        T = o.strings()
        preview_bundle(sys.argv[2])
    else:
        main()
