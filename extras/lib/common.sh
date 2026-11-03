# shellcheck shell=sh
# Sourced by every orcha script. Settings, colors and strings come from
# ~/.config/orcha/gen/env.sh, which `orcha apply` writes from config.json:
#   O_*  settings (O_IDE, O_GIT_APP, ...)     W_*  widgets on (1) / off (0)
#   C_*  colors as #hex                       R_*  colors as "r;g;b" for \033[38;2;...m
#   T_*  strings in the chosen language       FZF_DEFAULT_OPTS, BAT_THEME
ORCHA_CONFIG_DIR=${ORCHA_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/orcha}
PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if [ -f "$ORCHA_CONFIG_DIR/gen/env.sh" ]; then
  . "$ORCHA_CONFIG_DIR/gen/env.sh"
else
  echo "orcha: not set up yet, run: orcha setup" >&2
  exit 1
fi
S="$ORCHA_HOME/scripts"
mkdir -p "$ORCHA_CACHE"

# fg <R_color> <text>: colored text for fzf rows and previews
fg() { printf '\033[38;2;%sm%s\033[0m' "$1" "$2"; }

# q <text>: single-quote for a shell command line
q() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }

# send_path <path>: type a path into the pane the popup was opened from.
# Agent panes get "@path " (a file mention); plain shells get the bare path.
send_path() {
  pane=$HERDR_ACTIVE_PANE_ID
  [ -n "$pane" ] || return 0
  agent=$(herdr pane get "$pane" 2>/dev/null | jq -r '.result.pane.agent // empty')
  if [ -n "$agent" ]; then herdr pane send-text "$pane" "@$1 " >/dev/null
  else herdr pane send-text "$pane" "$1 " >/dev/null; fi
}

# in_pane <command>: run a terminal program in a new pane right of the focused one
in_pane() {
  out=$(herdr pane split --current --direction right --cwd "${HERDR_ACTIVE_PANE_CWD:-$PWD}" 2>/dev/null)
  pane=$(printf '%s' "$out" | jq -r '.result.pane.pane_id // empty')
  [ -n "$pane" ] && herdr pane run "$pane" "$1" >/dev/null
}

# open_editor <path> [line]: open a file in the chosen editor, at a line.
# Editors with a URL scheme get the file in their running window; their CLIs
# start a launcher first, which flashes a second window before handing over.
open_editor() {
  case $1 in /*) abs=$1 ;; *) abs="$PWD/$1" ;; esac
  line=$2
  enc=$(jq -rn --arg p "$abs" '$p | split("/") | map(@uri) | join("/")')
  case $O_IDE in
    cursor|vscode|windsurf|zed)
      url="$O_IDE://file$enc"
      [ -n "$line" ] && url="$url:$line"
      open "$url" 2>/dev/null || open -a "$O_IDE_APP" "$abs" ;;
    jetbrains)
      url="idea://open?file=$enc"
      [ -n "$line" ] && url="$url&line=$line"
      open "$url" 2>/dev/null || open -a "$O_IDE_APP" "$abs" ;;
    nvim)
      in_pane "nvim ${line:++$line }$(q "$abs")" ;;
    *)
      open "$abs" ;;
  esac
}
