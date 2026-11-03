#!/bin/sh
# Token usage report (popup), from ccusage: last 7 days and last 4 weeks.
#   usage-report.sh          the popup (fzf runs the report itself, so closing
#                            the popup also stops a report still loading)
#   usage-report.sh --body   just print the report
. "$(dirname "$0")/../lib/common.sh"
PATH="$HOME/.bun/bin:$PATH"
cd /tmp || exit 1
h() { printf '\033[1;38;2;%sm  %s\033[0m\n' "$R_accent" "$1"; }
if [ "$1" = --body ]; then
  command -v ccusage >/dev/null || { echo "$T_rep_no_ccusage"; exit 0; }
  h "$T_rep_daily"
  ccusage daily --offline --last 7 --compact --color
  echo; h "$T_rep_weekly"
  ccusage weekly --offline --start-of-week monday --last 4 --compact --color
  echo; h "$T_rep_claude_today"
  ccusage claude daily --offline --since "$(date +%Y%m%d)" --compact --color 2>/dev/null
  echo; h "$T_rep_codex_today"
  ccusage codex daily --offline --since "$(date +%Y%m%d)" --compact --color 2>/dev/null
  echo; fg "$R_muted" "  $T_rep_note"; echo
  exit 0
fi
fzf --ansi --no-input --no-sort --disabled --pointer=' ' --no-info \
    --bind "start:reload:$0 --body 2>&1" --bind 'enter:ignore' \
    --header="$T_rep_header" < /dev/null
