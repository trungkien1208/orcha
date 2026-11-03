"""`orcha` with nothing after it (or Cmd+, in herdr): the settings menu.

Each row shows what is set now; picking one opens its menu, a change is
applied right away, and Esc goes back (from the main menu: closes it).
"""
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import apply  # noqa: E402
import orchalib as o  # noqa: E402
import setup  # noqa: E402

LIMITS = os.path.join(o.ORCHA_HOME, "scripts", "limits.py")
BACK = "__back"


def T():
    return setup.T


def save(cfg):
    o.save_config(cfg)
    apply.apply(quiet=True)
    setup.T = o.strings(cfg["lang"])


def value(text):
    return o.fg(o.palette()["muted"], text)


def row(icon, label, current="", width=16):
    return f"{icon}  {label:<{width}}  {value(current)}"


def signed_in_email(acct):
    """The email an account is signed in with now (limits.py reads it the same way)."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("limits", LIMITS)
    limits = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(limits)
    try:
        return limits.email(acct["id"]) or ""
    except Exception:
        return ""


# ---------- sub-menus

def font_menu(cfg):
    rows = [(f, f) for f in o.FONTS]
    family = setup.menu(T()["s_font"], setup.mark_current(rows, cfg["font"]["family"]), esc=BACK)
    if family == BACK:
        return
    sizes = [(str(n), f"{n}") for n in range(12, 20)]
    size = setup.menu(T()["s_font_size"], setup.mark_current(sizes, str(cfg["font"]["size"])), esc=BACK)
    cfg["font"]["family"] = family
    if size != BACK:
        cfg["font"]["size"] = int(size)
    cask = o.FONTS.get(family)
    fonts = os.path.join(o.HOME, "Library", "Fonts")
    have = " ".join(os.listdir(fonts)).lower() if os.path.isdir(fonts) else ""
    if cask and family.split()[0].lower() not in have and shutil.which("brew"):
        setup.say(T()["s_note_font"].format(font=family), "muted")
        subprocess.run(["brew", "install", "--cask", cask], capture_output=True)
    save(cfg)


def widgets_menu(cfg):
    w = cfg["widgets"]
    while True:
        on = {k for k in o.WIDGETS if w[k]}
        rows = [(f"bundle:{b}", f"{icon}  {T()['b_' + b]}") for b, icon in (("focus", "🎯"), ("full", "✨"), ("minimal", "🍃"))]
        rows += [("-", value("─" * 40))]
        rows += [(k, f"{'◉' if w[k] else '○'}  {T()['w_' + k]}") for k in o.WIDGETS]
        pick = setup.menu(T()["s_widgets"] + f" · {len(on)}/{len(o.WIDGETS)}", rows, T()["m_widgets_hint"], esc=BACK)
        if pick == BACK:
            return
        if pick == "-":
            continue
        if pick.startswith("bundle:"):
            for k in o.WIDGETS:
                w[k] = k in o.BUNDLES[pick.split(":", 1)[1]]
        else:
            w[pick] = not w[pick]
        save(cfg)  # applied at once: the header / sidebar change while you look


def account_menu(cfg):
    while True:
        rows = []
        width = max([len(a.get("label") or a["id"]) for a in cfg["accounts"]] + [16])
        for a in cfg["accounts"]:
            mail = signed_in_email(a) or T()["s_not_signed_in"]
            rows.append((a["id"], row("👤", a.get("label") or a["id"], mail, width)))
        if shutil.which("claude"):
            rows.append(("add", "➕  " + T()["m_acc_add"]))
        pick = setup.menu(T()["m_accounts"], rows, T()["m_acc_hint"], esc=BACK)
        if pick == BACK:
            return
        if pick == "add":
            add_claude_account(cfg)
            continue
        acct = next(a for a in cfg["accounts"] if a["id"] == pick)
        actions = [("login", "🔑  " + T()["m_acc_login"]), ("usage", "📊  " + T()["m_acc_usage"])]
        if acct["kind"] == "claude":
            actions.insert(0, ("rename", "✎  " + T()["m_acc_rename"]))
            if acct.get("dir"):
                actions.append(("remove", "🗑  " + T()["m_acc_remove"]))
        what = setup.menu(acct.get("label") or pick, actions, esc=BACK)
        if what == "rename":
            taken = {a["id"].removeprefix("claude-") for a in cfg["accounts"] if a is not acct}
            if setup.pick_name(acct, taken, esc=BACK):
                save(cfg)
        elif what == "login":
            subprocess.run([LIMITS, "login", pick])
            acct["email"] = setup.claude_email(o.claude_dir(acct)) if acct["kind"] == "claude" else acct.get("email", "")
            save(cfg)
        elif what == "usage":
            subprocess.run([LIMITS, "show", pick])
            input(value("\n  Enter…"))
        elif what == "remove" and setup.yes_no(T()["m_acc_remove_ask"].format(name=acct.get("label")), default=False,
                                               hint=T()["m_acc_remove_hint"]):
            cfg["accounts"].remove(acct)
            save(cfg)


def add_claude_account(cfg):
    taken = {a["id"].removeprefix("claude-") for a in cfg["accounts"] if a["kind"] == "claude"}
    ideas = [n for n in ("personal", "work", "side", "team", "client") if n not in taken]
    rows = [(n, f"oc {n}") for n in ideas] + [("✎", "✎  " + T()["s_name_other"])]
    name = setup.menu(T()["m_acc_add"], rows, T()["s_name_hint"], esc=BACK)
    if name == BACK:
        return
    if name == "✎":
        name = re.sub(r"[^a-z0-9_-]+", "-", input(f"  {T()['s_name_type']}: ").strip().lower())
        if not name or name in taken:
            return
    acct = {"id": f"claude-{name}", "kind": "claude", "label": f"Claude · {name}", "email": "",
            "dir": f"~/.ai-accounts/claude/{name}"}
    cfg["accounts"].append(acct)
    o.save_config(cfg)
    subprocess.run([LIMITS, "login", acct["id"]])  # links the shared history, then signs in
    acct["email"] = setup.claude_email(o.claude_dir(acct))
    save(cfg)


# ---------- main menu

def main():
    cfg = o.load_config()
    setup.T = o.strings(cfg["lang"])
    names = {"en": "English", "vi": "Tiếng Việt"}
    while True:
        p = o.palette(cfg)
        on = sum(1 for k in o.WIDGETS if cfg["widgets"][k])
        accounts = ", ".join(a["id"].removeprefix("claude-") for a in cfg["accounts"]) or "—"
        ide = cfg.get("ide_app") or o.IDES[cfg["ide"]][0]
        rows = [
            ("palette", row("🎨", T()["s_palette"], T()["p_default"].split(":")[0] if p.get("native") else p["name"])),
            ("font", row("🔤", T()["s_font"], f"{cfg['font']['family']} · {cfg['font']['size']}")),
            ("widgets", row("🧩", T()["s_widgets"], T()["m_on_count"].format(n=on, total=len(o.WIDGETS)))),
            ("ide", row("✏️ ", T()["s_ide"], ide)),
            ("git", row("🌿", T()["m_git"], o.GIT_APPS[cfg["git_app"]][0])),
            ("accounts", row("👤", T()["m_accounts"], accounts)),
            ("azure", row("🔷", "Azure DevOps", T()["s_yes"] if os.path.exists(o.expand(cfg["tracker"]["azure_pat_file"])) else T()["s_no"])),
            ("lang", row("🌐", T()["m_lang"], names.get(cfg["lang"], cfg["lang"]))),
            ("-", value("─" * 44)),
            ("doctor", "🩺  " + T()["m_doctor"]),
            ("update", "🔄  " + T()["m_update"]),
            ("setup", "🪄  " + T()["m_setup"]),
            ("uninstall", "🗑   " + T()["m_uninstall"]),
        ]
        pick = setup.menu("🐋🍵 orcha", rows, T()["m_hint"], esc=BACK)
        if pick == BACK:
            return
        if pick == "palette":
            setup.ask_palette(cfg)
            save(cfg)
        elif pick == "font":
            font_menu(cfg)
        elif pick == "widgets":
            widgets_menu(cfg)
        elif pick == "ide":
            editors = setup.find_editors()
            if len(editors) > 1:
                setup.ask_editor(cfg, editors)
                save(cfg)
        elif pick == "git":
            setup.ask_git_app(cfg)
            save(cfg)
        elif pick == "accounts":
            account_menu(cfg)
        elif pick == "azure":
            setup.ask_azure(cfg)
            save(cfg)
        elif pick == "lang":
            setup.ask_lang(cfg)
            save(cfg)
        elif pick == "doctor":
            subprocess.run([sys.executable, "-I", os.path.join(o.ORCHA_HOME, "lib", "cli.py"), "doctor"])
            input(value("\n  Enter…"))
        elif pick == "update":
            subprocess.run(["git", "-C", o.ORCHA_HOME, "pull", "--ff-only"])
            save(cfg)
            input(value("\n  Enter…"))
        elif pick == "setup":
            os.execv(sys.executable, [sys.executable, "-I", os.path.join(o.ORCHA_HOME, "lib", "setup.py")])
        elif pick == "uninstall" and setup.yes_no(T()["m_uninstall_ask"], default=False, hint=T()["m_uninstall_hint"]):
            subprocess.run([sys.executable, "-I", os.path.join(o.ORCHA_HOME, "lib", "cli.py"), "uninstall"])
            return
        cfg = o.load_config()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
