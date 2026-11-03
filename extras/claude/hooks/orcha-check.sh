#!/bin/sh
# Run the repo's check (a typecheck, lint...) after Claude edits code.
#   orcha-check.sh mark  (PostToolUse Edit|Write|MultiEdit): remember a code file changed
#   orcha-check.sh stop  (Stop): run the check; on errors exit 2 so Claude fixes them
# The check is the repo rule "check": a command, "auto" (npm run typecheck when
# package.json has that script) or "" (none). At most 3 fix rounds per batch
# of edits, so it never loops forever.
here=$(dirname "$0")
. "$here/../../lib/common.sh" 2>/dev/null || exit 0
. "$here/repo-rules.sh"
mode=$1
input=$(cat)
sid=$(printf '%s' "$input" | jq -r '.session_id // "x"')
dir="${TMPDIR:-/tmp}/orcha-check"; mkdir -p "$dir"
dirty="$dir/$sid.dirty"; rounds="$dir/$sid.rounds"

case $mode in
  mark)
    path=$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.notebook_path // ""')
    case $path in *.ts|*.tsx|*.js|*.jsx|*.mjs|*.cjs|*.vue|*.svelte|*.py|*.go|*.rs) touch "$dirty" ;; esac
    exit 0 ;;
  stop)
    [ -f "$dirty" ] || exit 0
    cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
    top=$(git rev-parse --show-toplevel 2>/dev/null) || top=$PWD
    cd "$top" || exit 0
    check=$(repo_rules "$top" | jq -r '.check // ""')
    if [ "$check" = auto ]; then
      check=""
      jq -e '.scripts.typecheck' package.json >/dev/null 2>&1 && check="npm run typecheck --silent"
    fi
    [ -n "$check" ] || { rm -f "$dirty"; exit 0; }
    n=$(cat "$rounds" 2>/dev/null || echo 0)
    if [ "$n" -ge 3 ]; then
      rm -f "$dirty" "$rounds"
      echo "orcha: $T_check_gave_up" >&2
      exit 0
    fi
    out=$(sh -c "$check" 2>&1)
    if [ $? -eq 0 ]; then
      rm -f "$dirty" "$rounds"
      # the orcha Claude plugin reads this to cheer a pass
      # "<epoch> <fix rounds it took>": 0 rounds = passed on the first try
      echo "$(date +%s) $n" > "$ORCHA_CACHE/check-pass"
      exit 0
    fi
    echo $((n + 1)) > "$rounds"
    {
      printf "$T_check_failed\n\n" "$check" "$((n + 1))"
      printf '%s\n' "$out" | tail -c 6000
    } >&2
    exit 2 ;;
esac
