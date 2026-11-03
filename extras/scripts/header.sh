#!/bin/sh
# Header (herdr tab bar, right side): what the focused pane is working on.
#   󰄬 #1234 Task title… ● Active · ↑2 · ✎ 3 files +40/−2 · 🍅 18' · 󰚩 5h ███░░ 30% · ⛅ 31°
# Each part is a widget you can turn off in `orcha setup`. The task title and
# state come from GitHub / Azure DevOps (tracker.py, cached 30 minutes).
. "$(dirname "$0")/../lib/common.sh"
cwd=${HERDR_ACTIVE_PANE_CWD:-$PWD}
out=""

clip() { printf '%s' "$1" | awk -v n="$2" '{ if (length($0) > n) print substr($0, 1, n-1) "…"; else print }'; }

if [ "$W_task" = 1 ] && branch=$(git -C "$cwd" branch --show-current 2>/dev/null) && [ -n "$branch" ]; then
  id=$("$S/tracker.py" id "$branch")
  title=""; state=""
  if [ -n "$id" ]; then
    item=$("$S/tracker.py" get "$id" --repo "$cwd" | jq -c --arg id "$id" '.[$id] // empty' 2>/dev/null)
    if [ -n "$item" ]; then
      title=$(printf '%s' "$item" | jq -r '.title // empty')
      state=$(printf '%s' "$item" | jq -r '.state // empty')
    fi
  fi

  if [ -n "$title" ]; then
    out="󰄬 #$id $(clip "$title" 42)"
    [ -n "$state" ] && out="$out ● $state"
  elif [ -n "$id" ]; then
    out="󰄬 #$id"
  else
    out="⎇ $(clip "$branch" 36)"
  fi

  # push state: upstream, else origin/<branch> when it exists there
  remote=$(git -C "$cwd" rev-parse --abbrev-ref '@{u}' 2>/dev/null) ||
    remote=$(git -C "$cwd" rev-parse --verify --quiet "refs/remotes/origin/$branch" >/dev/null && echo "origin/$branch")
  if [ -z "$remote" ]; then
    out="$out · ⚠ $T_not_pushed"
  else
    n=$(git -C "$cwd" rev-list --count "$remote..HEAD" 2>/dev/null || echo 0)
    [ "$n" -gt 0 ] && out="$out · ↑$n"
  fi
  printf '%s' "$branch" | grep -Eq "$O_PROTECTED" && out="$out · 🔒"
fi

# from 18:00, once a day: a short end-of-day recap as a notification
stamp="$ORCHA_CACHE/recap-shown"
if [ "$W_recap" = 1 ] && [ "$(date +%H)" -ge 18 ] && [ "$(cat "$stamp" 2>/dev/null)" != "$(date +%F)" ]; then
  date +%F > "$stamp"
  ( msg=$("$S/worklog.py" recap-short 2>/dev/null)
    title=$T_recap_title; [ "$(date +%u)" = 5 ] && title=$T_recap_friday
    osascript -e "display notification \"$msg · $T_recap_more\" with title \"$title\"" >/dev/null 2>&1
    afplay /System/Library/Sounds/Glass.aiff >/dev/null 2>&1 ) &
fi

parts=""
[ "$W_changes" = 1 ] && parts="$parts
$(HERDR_ACTIVE_PANE_CWD=$cwd "$S/changes.sh" --summary 2>/dev/null)"
[ "$W_pomodoro" = 1 ] && parts="$parts
$("$S/pomodoro.sh" status 2>/dev/null)"
[ "$W_limits" = 1 ] && parts="$parts
$("$S/limits.py" short 2>/dev/null)"
[ "$W_weather" = 1 ] && parts="$parts
$("$S/weather.sh" 2>/dev/null)"
out=$(printf '%s\n%s\n' "$out" "$parts" | grep -v '^$' | awk 'NR > 1 { printf " · " } { printf "%s", $0 }')
printf '%s\n' "$out"
