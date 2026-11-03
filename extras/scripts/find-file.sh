#!/bin/sh
# Fuzzy-find files in the focused pane's folder.
# enter: open in your editor · ctrl-y: insert the path into the pane · esc: close
. "$(dirname "$0")/../lib/common.sh"
cd "${HERDR_ACTIVE_PANE_CWD:-$PWD}" || exit 1
# fzf starts fd itself (start:reload) so closing the popup stops a long scan too
out=$(fzf --bind 'start:reload:fd --type f --hidden --follow --exclude .git --exclude node_modules --exclude Library' \
      --expect=ctrl-y --prompt="  $T_find_prompt › " \
      --header="$T_find_header" \
      --preview='bat --color=always --style=numbers --line-range=:300 {}' \
      --preview-window='right,60%,border-left' < /dev/null)
key=$(printf '%s\n' "$out" | sed -n 1p)
file=$(printf '%s\n' "$out" | sed -n 2p)
[ -n "$file" ] || exit 0
if [ "$key" = ctrl-y ]; then send_path "$file"; else open_editor "$file"; fi
