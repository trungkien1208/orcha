#!/bin/sh
# Live search inside files (ripgrep) in the focused pane's folder.
# enter: open at that line in your editor · ctrl-y: insert the path · esc: close
. "$(dirname "$0")/../lib/common.sh"
cd "${HERDR_ACTIVE_PANE_CWD:-$PWD}" || exit 1
RG='rg --line-number --no-heading --color=always --smart-case --hidden --glob "!.git" --glob "!node_modules"'
out=$(: | fzf --ansi --disabled --expect=ctrl-y --prompt="  $T_grep_prompt › " \
      --header="$T_grep_header" \
      --bind "change:reload:sleep 0.1; $RG {q} || true" \
      --delimiter=: \
      --preview='bat --color=always --style=numbers --highlight-line {2} {1}' \
      --preview-window='right,60%,border-left,+{2}+3/3')
key=$(printf '%s\n' "$out" | sed -n 1p)
hit=$(printf '%s\n' "$out" | sed -n 2p)
[ -n "$hit" ] || exit 0
file=$(printf '%s' "$hit" | cut -d: -f1); line=$(printf '%s' "$hit" | cut -d: -f2)
if [ "$key" = ctrl-y ]; then send_path "$file"; else open_editor "$file" "$line"; fi
