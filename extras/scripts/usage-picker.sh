#!/bin/sh
# Account picker: plan limits per account, sign in, switch.
# click/enter: show in the header · ctrl-o: open in a new pane · ctrl-l: sign in · esc: close
#
# Sign-in runs after fzf exits (not inside fzf's execute): Claude's CLI crashes
# with "EINVAL kqueue" when started from inside fzf. The picker reopens after.
. "$(dirname "$0")/../lib/common.sh"
L="$S/limits.py"
while :; do
  out=$($L list | fzf --ansi --delimiter='\t' --with-nth=2 --no-sort \
      --expect=ctrl-l,ctrl-o \
      --prompt="󰚩 $T_acc_prompt › " --header="$T_acc_header" \
      --preview="$L show {1}" --preview-window='down,55%,border-top' \
      --bind "ctrl-t:execute($S/usage-report.sh)")
  key=$(printf '%s\n' "$out" | sed -n 1p)
  sel=$(printf '%s\n' "$out" | sed -n 2p | cut -f1)
  [ -n "$sel" ] || exit 0
  case $key in
    ctrl-l) clear; $L login "$sel" ;;          # then loop back to the picker
    ctrl-o) $L open "$sel"; exit 0 ;;
    *) $L select "$sel"
       herdr server reload-config >/dev/null 2>&1  # refresh the header right away
       exit 0 ;;
  esac
done
