"""orcha: a calm cup of terminal for your coding agents (Ghostty + herdr + Claude Code).

  orcha setup              pick colors, editor, accounts, widgets (re-run any time)
  orcha apply              rewrite the app configs from ~/.config/orcha/config.json
  orcha theme [name]       switch the color palette (no name: list them)
  orcha lang <en|vi>       switch the language
  orcha widget <name> [on|off]   turn one widget on / off (no name: list them)
  orcha doctor             check that everything orcha needs is there
  orcha edit               open config.json in your editor, apply after
  orcha uninstall          put your old Ghostty / herdr / Claude configs back
"""
import json
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import apply  # noqa: E402
import orchalib as o  # noqa: E402

DEPS = [  # (command, brew name, needed for)
    ("ghostty", "--cask ghostty", "the terminal"),
    ("herdr", "herdr", "panes, tabs, agent sidebar"),
    ("fzf", "fzf", "every popup"),
    ("fd", "fd", "find file"),
    ("rg", "ripgrep", "search text"),
    ("bat", "bat", "previews"),
    ("jq", "jq", "scripts"),
    ("yazi", "yazi", "file browser"),
    ("lame", "lame", "sounds widget"),
    ("gh", "gh", "GitHub task titles + PR links"),
    ("ccusage", "ccusage", "token report"),
]


def doctor():
    cfg = o.load_config()
    p = o.palette(cfg)
    good, bad, warn = o.fg(p["green"], "✓"), o.fg(p["red"], "✗"), o.fg(p["yellow"], "!")
    print(o.fg(p["accent"], "  orcha doctor", bold=True))
    print(f"  {good if os.path.exists(o.CONFIG_FILE) else bad} config  {o.CONFIG_FILE.replace(o.HOME, '~')}")
    for cmd, brew, why in DEPS:
        found = shutil.which(cmd) or (cmd == "ghostty" and os.path.exists("/Applications/Ghostty.app"))
        mark = good if found else (warn if cmd in ("lame", "gh", "ccusage", "yazi") else bad)
        hint = "" if found else o.fg(p["muted"], f"   brew install {brew}")
        print(f"  {mark} {cmd:<8} {o.fg(p['muted'], why)}{hint}")
    fonts = os.path.join(o.HOME, "Library", "Fonts")
    have = " ".join(os.listdir(fonts)).lower().replace(" ", "") if os.path.isdir(fonts) else ""
    fam = cfg["font"]["family"].lower().replace(" ", "").replace("nerdfont", "")
    font_ok = fam.split("nf")[0][:8] in have
    print(f"  {good if font_ok else warn} font     {cfg['font']['family']}"
          + ("" if font_ok else o.fg(p["muted"], f"   brew install --cask {o.FONTS.get(cfg['font']['family'], '')}")))
    if cfg["widgets"]["logos"]:
        ok = os.path.exists(os.path.join(fonts, "AgentIcons.ttf"))
        print(f"  {good if ok else warn} logos    Agent Icons" + ("" if ok else o.fg(p["muted"], "   orcha logos")))
    # each account: its picker row ("<id>\t<mark> <label (email)>  <summary>"), "—" when it has no data
    rows = subprocess.run([os.path.join(o.ORCHA_HOME, "scripts", "limits.py"), "list"],
                          capture_output=True, text=True, timeout=60).stdout.splitlines()
    for row in rows:
        aid, _, rest = row.partition("\t")
        label = rest[2:].rstrip()
        is_bad = label.endswith("—") or "⚠" in label
        hint = o.fg(p["muted"], f"   oc-login {aid.removeprefix('claude-')}") if is_bad else ""
        print(f"  {warn if is_bad else good} account  {' '.join(label.split())}{hint}")
    # the main Ghostty config setting what orcha sets: a theme hides orcha's
    # "default" palette, a second custom-shader stacks on orcha's
    main = next((f for f in apply.GHOSTTY_MAIN if os.path.exists(f)), None)
    if main:
        own = [ln.split("=")[0].strip() for ln in open(main).read().split(apply.MARK)[0].splitlines()
               if re.match(r"\s*(theme|custom-shader|font-family)\s*=", ln)]
        if own:
            print(f"  {warn} ghostty  {main.replace(o.HOME, '~')} also sets {', '.join(sorted(set(own)))}"
                  + o.fg(p["muted"], "   (orcha's own lines override, shaders stack: remove yours?)"))
    hooks = json.dumps(json.load(open(apply.CLAUDE_SETTINGS))) if os.path.exists(apply.CLAUDE_SETTINGS) else ""
    print(f"  {good if 'orcha-guard.sh' in hooks else warn} claude   guard + check hooks in ~/.claude/settings.json")
    if shutil.which("claude"):
        listed = subprocess.run(["claude", "plugin", "list"], capture_output=True, text=True).stdout
        print(f"  {good if 'orcha' in listed else warn} claude   orcha plugin (task card, /branches, /pr-link)"
              + ("" if "orcha" in listed else o.fg(p["muted"], "   orcha plugin")))


def uninstall():
    restored = []
    for main in apply.GHOSTTY_MAIN:
        if os.path.exists(main):
            with open(main) as fh:
                lines = fh.read().splitlines()
            keep = [ln for ln in lines if not ln.startswith(apply.MARK) and "orcha.ghostty" not in ln]
            if keep != lines:
                with open(main, "w") as fh:
                    fh.write("\n".join(keep).rstrip("\n") + "\n")
                restored.append(main)
    herdr_cfg = os.path.join(apply.HERDR_DIR, "config.toml")
    if os.path.exists(herdr_cfg + ".pre-orcha"):
        shutil.move(herdr_cfg + ".pre-orcha", herdr_cfg)
        restored.append(herdr_cfg)
    elif "--all" in sys.argv and generated_by_orcha(herdr_cfg):
        os.remove(herdr_cfg)  # there was no herdr config before orcha
        restored.append(herdr_cfg + " (removed: orcha made it)")
    apply.remove_claude_hooks()
    apply.claude_theme(False)
    restored.append(apply.CLAUDE_SETTINGS + " (orcha hooks removed)")
    for f in ("orcha.ghostty", "themes/orcha", "shaders/orcha-trail.glsl"):
        try:
            os.remove(os.path.join(apply.GHOSTTY_DIR, f))
        except OSError:
            pass
    apply.remove_codex_hooks()
    restored.append(apply.CODEX_HOOKS + " (orcha hook removed)")
    if "--all" in sys.argv:
        restored += remove_everything_else()
    if shutil.which("herdr"):
        subprocess.run(["herdr", "server", "reload-config"], capture_output=True)
    print("restored:\n  " + "\n  ".join(r.replace(o.HOME, "~") for r in restored))
    if "--all" not in sys.argv:
        print("kept: ~/.config/orcha (your answers), the zsh line in ~/.zshrc, fonts.")
        print("Claude plugin: claude plugin uninstall orcha@orcha")


def generated_by_orcha(path):
    try:
        with open(path) as fh:
            return "generated by `orcha apply`" in fh.read()
    except OSError:
        return False


def remove_empty_files_and_dirs(paths):
    """Config files orcha left empty, and folders it made that are empty now."""
    for path in paths:
        try:
            if os.path.isfile(path) and not open(path).read().strip():
                os.remove(path)
            if os.path.isdir(path) and not os.listdir(path):
                os.rmdir(path)
        except OSError:
            pass


def remove_everything_else():
    """`uninstall --all` (the orcha app's uninstaller): also the shell lines,
    the logo font, the Claude plugin and the link to the scripts."""
    done = []
    zshrc = os.path.join(o.HOME, ".zshrc")
    try:
        with open(zshrc) as fh:
            lines = fh.read().splitlines()
        keep = [ln for ln in lines if not ln.rstrip().endswith("# orcha")]
        if keep != lines:
            with open(zshrc, "w") as fh:
                fh.write("\n".join(keep).rstrip("\n") + "\n")
            done.append(zshrc + " (orcha lines removed)")
    except OSError:
        pass
    font = os.path.join(o.HOME, "Library", "Fonts", "AgentIcons.ttf")
    if os.path.exists(font):
        os.remove(font)
        done.append(font + " (removed)")
    if shutil.which("claude"):
        r = subprocess.run(["claude", "plugin", "uninstall", "orcha@orcha", "--scope", "user"], capture_output=True)
        subprocess.run(["claude", "plugin", "marketplace", "remove", "orcha"], capture_output=True)
        if r.returncode == 0:
            done.append("Claude plugin orcha@orcha (uninstalled)")
    scripts = os.path.join(o.CONFIG_DIR, "scripts")
    if os.path.islink(scripts):
        os.remove(scripts)
    # backups orcha made before its first edit: the lines it added are out now
    for main in [*apply.GHOSTTY_MAIN, apply.CODEX_HOOKS, apply.CLAUDE_SETTINGS]:
        if os.path.exists(main + ".pre-orcha"):
            os.remove(main + ".pre-orcha")
    remove_empty_files_and_dirs([zshrc, *apply.GHOSTTY_MAIN, os.path.join(apply.GHOSTTY_DIR, "themes"),
                                 os.path.join(apply.GHOSTTY_DIR, "shaders"), apply.GHOSTTY_DIR, apply.HERDR_DIR])
    return done


def set_and_apply(key, value):
    cfg = o.load_config()
    cfg[key] = value
    o.save_config(cfg)
    apply.apply()


def main():
    args = sys.argv[1:]
    cmd = args[0] if args else "help"
    if cmd == "apply":
        apply.apply()
    elif cmd == "theme":
        if len(args) < 2 or args[1] not in o.palette_names():
            print("\n".join(o.palette_names()))
        else:
            set_and_apply("palette", args[1])
    elif cmd == "font":
        # orcha font "<family>" [size]
        cfg = o.load_config()
        if len(args) < 2:
            print(f"{cfg['font']['family']} {cfg['font']['size']}")
        else:
            cfg["font"]["family"] = args[1]
            if len(args) > 2 and args[2].isdigit():
                cfg["font"]["size"] = int(args[2])
            o.save_config(cfg)
            apply.apply()
    elif cmd == "ligatures":
        cfg = o.load_config()
        if len(args) < 2 or args[1] not in ("on", "off"):
            print("on" if cfg["font"].get("ligatures", True) else "off")
        else:
            cfg["font"]["ligatures"] = args[1] == "on"
            o.save_config(cfg)
            apply.apply()
    elif cmd == "lang":
        if len(args) < 2 or args[1] not in o.languages():
            print("\n".join(o.languages()))
        else:
            set_and_apply("lang", args[1])
    elif cmd == "widget":
        cfg = o.load_config()
        if len(args) < 2 or args[1] not in o.WIDGETS:
            for k in o.WIDGETS:
                print(f"{'on ' if cfg['widgets'][k] else 'off'}  {k}")
            return
        cfg["widgets"][args[1]] = (args[2] == "on") if len(args) > 2 else not cfg["widgets"][args[1]]
        o.save_config(cfg)
        apply.apply()
    elif cmd == "doctor":
        doctor()
    elif cmd == "uninstall":
        uninstall()
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
