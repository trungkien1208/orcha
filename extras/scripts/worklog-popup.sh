#!/bin/sh
# Show worklog.py output in a popup. fzf runs the (slow) report itself, so
# closing the popup stops it.
#   worklog-popup.sh standup   also copies the standup to the clipboard
#   worklog-popup.sh recap
. "$(dirname "$0")/../lib/common.sh"
W="$S/worklog.py"
case $1 in
  standup) gen="$W standup > \"\${TMPDIR:-/tmp}/standup.md\" && pbcopy < \"\${TMPDIR:-/tmp}/standup.md\" && cat \"\${TMPDIR:-/tmp}/standup.md\""
           head=$T_standup_header ;;
  recap)   gen="$W recap"; head=$T_recap_header ;;
  *) exit 1 ;;
esac
fzf --ansi --no-input --no-sort --disabled --pointer=' ' --no-info \
    --bind "start:reload:$gen" --bind 'enter:ignore' --header="$head" < /dev/null
