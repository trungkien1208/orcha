"""Shared bits for orcha's Python scripts: paths, config, palette, strings.

Config lives in ~/.config/orcha/config.json (written by `orcha setup`); every
key has a default here, so a partial file works. Scripts import this with

    sys.path.insert(0, os.path.join(os.path.dirname(os.path.realpath(__file__)), "..", "lib"))
    import orchalib as o
"""
import copy
import json
import os

HOME = os.path.expanduser("~")
ORCHA_HOME = os.path.realpath(os.path.join(os.path.dirname(os.path.realpath(__file__)), ".."))
CONFIG_DIR = os.path.join(os.environ.get("XDG_CONFIG_HOME") or os.path.join(HOME, ".config"), "orcha")
CONFIG_FILE = os.path.join(CONFIG_DIR, "config.json")
GEN_DIR = os.path.join(CONFIG_DIR, "gen")
CACHE = os.path.join(HOME, ".cache", "orcha")

DEFAULTS = {
    "lang": "en",
    "palette": "dark-rose",
    "accent": "",  # "#rrggbb" overrides the palette's accent
    "font": {"family": "JetBrainsMono Nerd Font", "size": 15},
    "ide": "cursor",  # cursor | vscode | windsurf | zed | jetbrains | nvim
    "git_app": "none",  # sourcetree | fork | github-desktop | tower | gitkraken | lazygit | none
    # [{"id", "kind": claude|codex|cursor, "label", "email", "dir"}]; the first
    # Claude account uses ~/.claude (dir null), others their own folder
    "accounts": [],
    "tracker": {
        # the task id in a branch name: feature/1234-login, 1234_fix, ...
        "task_pattern": r"(?:^|[/_#-])(\d{2,7})(?=[/_-]|$)",
        # Azure DevOps token: env AZURE_DEVOPS_PAT, else this file's
        # AZURE_DEVOPS_PAT= line; {repo} is the repo's root folder
        "azure_pat_file": "~/.config/orcha/secrets.env",
    },
    "repo": {
        "branch_prefix": "",  # your branches, for /branches (e.g. "kim/")
        "base": "",  # branch PRs go into; empty: the remote's default branch
        "protected": r"^(release/.+|develop|main|master)$",
        "check": "auto",  # command run when Claude stops; auto: npm run typecheck if the repo has it
        "block": [],  # regexes of shell commands Claude must not run
    },
    "weather_city": "",  # empty: located by IP
    "widgets": {
        "task": True, "changes": True, "pomodoro": True, "limits": True, "weather": True, "clock": True,
        "sidebar_groups": True, "logos": True, "tab_names": True, "cursor_trail": True,
        "card": False, "recap": True, "sounds": False, "cheers": False, "claude_colors": True,
    },
}

WIDGETS = list(DEFAULTS["widgets"])

# In order of preference: setup picks the first one installed.
IDES = {  # id: (label, app name for `open -a`; empty: a terminal program)
    "cursor": ("Cursor", "Cursor"),
    "vscode": ("VS Code", "Visual Studio Code"),
    "windsurf": ("Windsurf", "Windsurf"),
    "zed": ("Zed", "Zed"),
    "jetbrains": ("JetBrains (IntelliJ, WebStorm, ...)", "IntelliJ IDEA"),
    "nvim": ("Neovim (in a pane)", ""),
}
IDE_CLI = {"nvim": "nvim"}
GIT_APPS = {
    "sourcetree": ("Sourcetree", "Sourcetree"),
    "fork": ("Fork", "Fork"),
    "github-desktop": ("GitHub Desktop", "GitHub Desktop"),
    "tower": ("Tower", "Tower"),
    "gitkraken": ("GitKraken", "GitKraken"),
    "lazygit": ("lazygit (in a pane)", ""),
    "none": ("none", ""),
}
GIT_CLI = {"lazygit": "lazygit"}
# Coding agents orcha knows: herdr's agent id: (command, name, icon without the
# logo font). Plan limits exist for claude, codex and cursor only.
AGENTS = {
    "claude": ("claude", "Claude", "✳"),
    "codex": ("codex", "Codex", "◎"),
    "cursor": ("cursor-agent", "Cursor", "▲"),
    "gemini": ("gemini", "Gemini", "✦"),
    "opencode": ("opencode", "OpenCode", "◇"),
    "amp": ("amp", "Amp", "⚡"),
    "droid": ("droid", "Droid", "◆"),
    "copilot": ("copilot", "Copilot", "◈"),
    "aider": ("aider", "Aider", "◉"),
    "qwen": ("qwen", "Qwen", "◐"),
    "goose": ("goose", "Goose", "🪿"),
    "crush": ("crush", "Crush", "✧"),
}
LIMIT_KINDS = ("claude", "codex", "cursor")
# where agent installers put their command when it is not on PATH yet
AGENT_DIRS = ("~/.local/bin", "~/.claude/local", "/opt/homebrew/bin", "/usr/local/bin",
              "~/.npm-global/bin", "~/.bun/bin", "~/.volta/bin", "~/.cargo/bin")


def agent_path(cmd):
    """The agent's command: on PATH, else in a usual install folder."""
    import shutil
    found = shutil.which(cmd)
    if found:
        return found
    for folder in AGENT_DIRS:
        path = os.path.join(os.path.expanduser(folder), cmd)
        if os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None


def installed_agents():
    return [aid for aid, (cmd, _, _) in AGENTS.items() if agent_path(cmd)]


def claude_email(folder=None):
    """The signed-in email of a Claude config folder (~/.claude by default)."""
    path = os.path.join(folder, ".claude.json") if folder else os.path.join(HOME, ".claude.json")
    try:
        with open(path) as fh:
            return (json.load(fh).get("oauthAccount") or {}).get("emailAddress") or ""
    except (OSError, ValueError):
        return ""


def with_detected_accounts(accounts):
    """`accounts` plus one for each of Claude / Codex / Cursor installed but
    not set up yet, so a new agent shows up without running setup again."""
    have = {a["kind"] for a in accounts}
    out = list(accounts)
    for kind in LIMIT_KINDS:
        if kind in have or not agent_path(AGENTS[kind][0]):
            continue
        if kind == "claude":
            out.append({"id": "claude-main", "kind": "claude", "label": "Claude",
                        "email": claude_email(), "dir": None})
        else:
            out.append({"id": kind, "kind": kind, "label": AGENTS[kind][1], "email": "", "dir": None})
    return out
# Widget bundles for setup; "custom" toggles them one by one.
BUNDLES = {
    "focus": ["claude_colors", "task", "changes", "limits", "clock", "pomodoro", "sidebar_groups", "logos", "tab_names"],
    "full": ["claude_colors", "task", "changes", "limits", "clock", "pomodoro", "weather", "sidebar_groups", "logos", "tab_names",
             "cursor_trail", "recap", "sounds", "cheers"],
    "minimal": ["claude_colors", "sidebar_groups", "logos", "tab_names", "clock"],
}
FONTS = {  # family: Homebrew cask
    "JetBrainsMono Nerd Font": "font-jetbrains-mono-nerd-font",
    "Maple Mono NF": "font-maple-mono-nf",
    "FiraCode Nerd Font": "font-fira-code-nerd-font",
    "CaskaydiaCove Nerd Font": "font-caskaydia-cove-nerd-font",
}


def _merge(base, over):
    out = copy.deepcopy(base)
    for k, v in (over or {}).items():
        out[k] = _merge(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out


def load_config():
    try:
        with open(CONFIG_FILE) as fh:
            user = json.load(fh)
    except (OSError, ValueError):
        user = {}
    return _merge(DEFAULTS, user)


def save_config(cfg):
    os.makedirs(CONFIG_DIR, exist_ok=True)
    tmp = CONFIG_FILE + ".tmp"
    with open(tmp, "w") as fh:
        json.dump(cfg, fh, indent=2, ensure_ascii=False)
        fh.write("\n")
    os.replace(tmp, CONFIG_FILE)


def expand(path):
    return os.path.expanduser(path) if path else path


def palette_names():
    d = os.path.join(ORCHA_HOME, "palettes")
    return sorted(f[:-5] for f in os.listdir(d) if f.endswith(".json"))


def palette(cfg=None):
    cfg = cfg or load_config()
    name = cfg["palette"] if cfg["palette"] in palette_names() else DEFAULTS["palette"]
    with open(os.path.join(ORCHA_HOME, "palettes", name + ".json")) as fh:
        p = json.load(fh)
    if cfg.get("accent"):
        p["accent"] = p["cursor"] = cfg["accent"]
    return p


def rgb(hex_color):
    h = hex_color.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def strings(lang=None):
    """English strings with the chosen language on top (missing keys stay English)."""
    lang = lang or load_config()["lang"]
    with open(os.path.join(ORCHA_HOME, "i18n", "en.json")) as fh:
        out = json.load(fh)
    if lang != "en":
        try:
            with open(os.path.join(ORCHA_HOME, "i18n", lang + ".json")) as fh:
                out.update(json.load(fh))
        except OSError:
            pass
    return out


def languages():
    return sorted(f[:-5] for f in os.listdir(os.path.join(ORCHA_HOME, "i18n")) if f.endswith(".json"))


def app_installed(name):
    return any(os.path.isdir(os.path.join(d, name + ".app"))
               for d in ("/Applications", os.path.join(HOME, "Applications"), "/Applications/Setapp"))


def claude_dir(acct):
    """CLAUDE_CONFIG_DIR of a Claude account; None for the default ~/.claude."""
    return expand(acct.get("dir")) or None


def fg(color, s, bold=False):
    r, g, b = rgb(color) if isinstance(color, str) else color
    return f"\033[{'1;' if bold else ''}38;2;{r};{g};{b}m{s}\033[0m"
