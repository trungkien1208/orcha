#!/bin/sh
# Claude Code guards (settings hooks, so no plugin can skip them).
#   orcha-guard.sh pre   (PreToolUse Bash|Edit|Write|MultiEdit|NotebookEdit)
#     - deny shell commands matching the repo's "block" patterns
#     - ask before the first edit / risky git command on a protected branch
#   orcha-guard.sh post  (PostToolUse, same tools): the call ran, so this branch
#     is confirmed for this session and is not asked about again
here=$(dirname "$0")
. "$here/../../lib/common.sh" 2>/dev/null || exit 0
. "$here/repo-rules.sh"
mode=$1
input=$(cat)
tool=$(printf '%s' "$input" | jq -r '.tool_name // ""')
cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // ""')
sid=$(printf '%s' "$input" | jq -r '.session_id // "x"')
cd "${CLAUDE_PROJECT_DIR:-.}" 2>/dev/null || exit 0
top=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
branch=$(git branch --show-current 2>/dev/null)
rules=$(repo_rules "$top")
protected=$(printf '%s' "$rules" | jq -r '.protected // ""')
dir="${TMPDIR:-/tmp}/orcha-guard"; mkdir -p "$dir"
ok="$dir/$sid"

is_risky() {
  case $tool in
    Edit|Write|MultiEdit|NotebookEdit) return 0 ;;
    Bash) printf '%s' "$cmd" | grep -Eq '(^|[^[:alnum:]_])git[[:space:]]+(commit|push|merge|rebase|reset|cherry-pick|revert)([^[:alnum:]_]|$)' ;;
    *) return 1 ;;
  esac
}
is_protected() { [ -n "$protected" ] && [ -n "$branch" ] && printf '%s' "$branch" | grep -Eq "$protected"; }
confirmed() { [ -f "$ok" ] && grep -Fxq "$branch" "$ok"; }

case $mode in
  pre)
    if [ "$tool" = Bash ]; then
      # each block rule: "regex" or {"pattern": "regex", "reason": "why"}
      hit=$(printf '%s' "$rules" | jq -c '.block[]? | if type == "string" then {pattern: ., reason: ""} else . end' |
        while read -r rule; do
          pat=$(printf '%s' "$rule" | jq -r '.pattern')
          if printf '%s' "$cmd" | grep -Eq -- "$pat"; then printf '%s' "$rule"; break; fi
        done)
      if [ -n "$hit" ]; then
        jq -n --argjson r "$hit" --arg t "$T_guard_blocked" \
          '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "deny",
            permissionDecisionReason: ($t + " (" + $r.pattern + ")" + (if $r.reason != "" then ". " + $r.reason else "" end))}}'
        exit 0
      fi
    fi
    if is_risky && is_protected && ! confirmed; then
      what=$T_guard_edit; [ "$tool" = Bash ] && what=$T_guard_git
      jq -n --arg r "⚠ $(printf "$T_guard_protected" "$branch" "$what")" \
        '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "ask", permissionDecisionReason: $r}}'
    fi
    exit 0 ;;
  post)
    if is_risky && is_protected && ! confirmed; then echo "$branch" >> "$ok"; fi
    exit 0 ;;
esac
