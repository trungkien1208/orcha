#!/bin/sh
# Pomodoro for the header.
#   pomodoro.sh start [minutes]   focus block (25 by default), then a 5 minute break
#   pomodoro.sh stop
#   pomodoro.sh status            "🍅 18'" / "☕ 3'" for the header, nothing when off
# A detached timer rings at the end of each block (notification + sound).
. "$(dirname "$0")/../lib/common.sh"
F="$ORCHA_CACHE/pomodoro"   # "<end epoch> <work|break>"
self=$0

notify() {
  osascript -e "display notification \"$2\" with title \"$1\"" >/dev/null 2>&1
  afplay /System/Library/Sounds/Glass.aiff >/dev/null 2>&1 &
}

schedule() {  # <minutes> <phase>
  end=$(( $(date +%s) + $1 * 60 ))
  echo "$end $2" > "$F"
  nohup sh -c "sleep $(( $1 * 60 )); '$self' ring $end" >/dev/null 2>&1 &
}

case $1 in
  start) schedule "${2:-25}" work; notify "$T_pomo_start_title" "${2:-25} $T_pomo_start_body" ;;
  stop)  rm -f "$F" ;;
  ring)
    # ignore a timer whose block was stopped or replaced
    [ -f "$F" ] && [ "$(cut -d' ' -f1 "$F")" = "$2" ] || exit 0
    if [ "$(cut -d' ' -f2 "$F")" = work ]; then
      schedule 5 break
      notify "$T_pomo_break_title" "$T_pomo_break_body"
    else
      rm -f "$F"
      notify "$T_pomo_back_title" "$T_pomo_back_body"
    fi ;;
  status)
    [ -f "$F" ] || exit 0
    read -r end phase < "$F"
    left=$(( end - $(date +%s) ))
    [ "$left" -gt 0 ] || exit 0
    icon="🍅"; [ "$phase" = break ] && icon="☕"
    echo "$icon $(( (left + 59) / 60 ))'" ;;
  *) echo "usage: pomodoro.sh start [min] | stop | status" >&2; exit 1 ;;
esac
