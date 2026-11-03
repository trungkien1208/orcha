#!/bin/sh
# What changed in the focused pane's repo (uncommitted, staged or not, new files):
# one row per file with +/- lines, the diff on the right.
#   changes.sh            the popup
#   changes.sh --rows     the rows (fzf reloads them)
#   changes.sh --diff F   the colored diff of one file (the preview)
#   changes.sh --summary  "✎ 7 files +120/−30" for the header, nothing when clean
# enter: open the file at its first change · ctrl-y: insert path · ctrl-r: refresh
. "$(dirname "$0")/../lib/common.sh"
dir=${HERDR_ACTIVE_PANE_CWD:-$PWD}
root=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || { [ "$1" = --summary ] && exit 0; echo "$T_not_git: $dir"; exit 0; }
cd "$root" || exit 1
self=$0

rows() {
  # tracked changes (staged + unstaged) against HEAD, with line counts
  git diff HEAD --numstat 2>/dev/null | while IFS="$(printf '\t')" read -r add del path; do
    st=$(git status --porcelain=v1 -- "$path" | cut -c1-2 | tr -d ' ')
    case $st in D*) icon=$(fg "$R_red" ✗) ;; A*) icon=$(fg "$R_green" +) ;; *) icon=$(fg "$R_yellow" ●) ;; esac
    [ "$add" = - ] && add=bin && del=bin
    printf '%s\t%s  %s %s  %s\n' "$path" "$icon" "$(fg "$R_green" "$(printf '%5s' "+$add")")" \
      "$(fg "$R_red" "$(printf '%5s' "-$del")")" "$path"
  done
  # new files git does not track yet
  git ls-files --others --exclude-standard | while read -r path; do
    n=$(wc -l < "$path" 2>/dev/null | tr -d ' ')
    printf '%s\t%s  %s %5s  %s %s\n' "$path" "$(fg "$R_blue" '?')" "$(fg "$R_green" "$(printf '%5s' "+${n:-0}")")" "" \
      "$path" "$(fg "$R_muted" "($T_new_file)")"
  done
}

case $1 in
  --rows) rows; exit 0 ;;
  --diff)
    if git ls-files --error-unmatch -- "$2" >/dev/null 2>&1; then
      git -c color.diff=always diff HEAD -- "$2"
    else
      bat --color=always --style=numbers -- "$2" 2>/dev/null || cat -- "$2"
    fi
    exit 0 ;;
  --summary)
    files=$(git status --porcelain=v1 -uall 2>/dev/null | wc -l | tr -d ' ')
    [ "$files" -gt 0 ] || exit 0
    stat=$(git diff HEAD --shortstat 2>/dev/null)
    add=$(printf '%s' "$stat" | grep -oE '[0-9]+ insertion' | grep -oE '[0-9]+')
    del=$(printf '%s' "$stat" | grep -oE '[0-9]+ deletion' | grep -oE '[0-9]+')
    echo "✎ $files $T_files +${add:-0}/−${del:-0}"
    exit 0 ;;
esac

branch=$(git branch --show-current)
count=$(git status --porcelain=v1 -uall | wc -l | tr -d ' ')
out=$(fzf --ansi --delimiter='\t' --with-nth=2 --no-sort --expect=ctrl-y \
      --bind "start:reload:$self --rows" --bind "ctrl-r:reload:$self --rows" \
      --prompt="  $T_changes_prompt › " \
      --header="$(basename "$root") ⎇ $branch · $count $T_changes_uncommitted · $T_changes_header" \
      --preview="$self --diff {1}" --preview-window='right,62%,border-left,wrap' < /dev/null)
key=$(printf '%s\n' "$out" | sed -n 1p)
file=$(printf '%s\n' "$out" | sed -n 2p | cut -f1)
[ -n "$file" ] || exit 0
if [ "$key" = ctrl-y ]; then
  send_path "$file"
else
  # first changed line of the file, so the editor lands on the change
  line=$(git diff -U0 HEAD -- "$file" 2>/dev/null | sed -n 's/^@@ -[0-9,]* +\([0-9]*\).*/\1/p' | head -1)
  open_editor "$file" "${line:-1}"
fi
