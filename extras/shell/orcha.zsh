# orcha: several Claude Code accounts, one shared conversation history.
# Sourced from ~/.zshrc (orcha setup adds the line).
#
# Claude accounts come from `accounts` in ~/.config/orcha/config.json. The
# first one uses the normal ~/.claude; each other one gets its own folder
# that links everything from ~/.claude except the sign-in files, so all
# accounts share conversations: start with one, `oc <other> --continue`.
#
#   oc <account> [claude args]   run claude with that account (oc work, oc personal)
#   oc-use <account>             switch this shell, plain `claude` follows
#   oc-who                       which account this shell uses
#   oc-login <account>           sign an account in (also codex / cursor)

ORCHA_HOME=${ORCHA_HOME:-${${(%):-%x}:A:h:h}}
ORCHA_CONFIG=${XDG_CONFIG_HOME:-$HOME/.config}/orcha/config.json

# _oc_account <name>: "<id>\t<dir or empty>" of a Claude account, matched by id
# or by id without its "claude-" start
_oc_account() {
  jq -r --arg n "$1" '.accounts[]? | select(.kind == "claude")
    | select(.id == $n or .id == ("claude-" + $n)) | "\(.id)\t\(.dir // "")"' "$ORCHA_CONFIG" 2>/dev/null | head -1
}

_oc_names() {
  jq -r '[.accounts[]? | select(.kind == "claude") | .id | sub("^claude-"; "")] | join(" | ")' "$ORCHA_CONFIG" 2>/dev/null
}

# _oc_dir <name>: the account's CLAUDE_CONFIG_DIR ("" for ~/.claude), linking
# the shared files in first
_oc_dir() {
  local row; row=$(_oc_account "$1")
  [[ -n $row ]] || { echo "orcha: no Claude account '$1' (have: $(_oc_names))" >&2; return 1; }
  local d=${row#*$'\t'}
  [[ -z $d ]] && return 0
  d=${~d}
  [[ -d $d ]] || echo "orcha: new account folder, sign in once: oc-login $1" >&2
  "$ORCHA_HOME/scripts/limits.py" link "${row%%$'\t'*}" >/dev/null 2>&1
  echo "$d"
}

oc() {
  local a=${1:?usage: oc <account> [claude args]   (accounts: $(_oc_names))}; shift
  local d; d=$(_oc_dir "$a") || return 1
  if [[ -z $d ]]; then (unset CLAUDE_CONFIG_DIR; command claude "$@")
  else CLAUDE_CONFIG_DIR="$d" command claude "$@"; fi
}

oc-use() {
  local a=${1:?usage: oc-use <account>   (accounts: $(_oc_names))}
  local d; d=$(_oc_dir "$a") || return 1
  if [[ -z $d ]]; then unset CLAUDE_CONFIG_DIR; else export CLAUDE_CONFIG_DIR="$d"; fi
  oc-who
}

oc-who() {
  local f="$HOME/.claude.json" email
  [[ -n $CLAUDE_CONFIG_DIR ]] && f="$CLAUDE_CONFIG_DIR/.claude.json"
  email=$(jq -r '.oauthAccount.emailAddress // "not signed in"' "$f" 2>/dev/null || echo "not signed in")
  echo "claude: ${CLAUDE_CONFIG_DIR:-~/.claude} ($email)"
}

oc-login() {
  local a=${1:?usage: oc-login <account | codex | cursor>}
  local row id; row=$(_oc_account "$a"); id=${row%%$'\t'*}
  "$ORCHA_HOME/scripts/limits.py" login "${id:-$a}"
}

_oc_complete() { compadd ${(s: | :)$(_oc_names)} codex cursor }
(( $+functions[compdef] )) && compdef _oc_complete oc oc-use oc-login
