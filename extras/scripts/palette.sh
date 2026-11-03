#!/bin/sh
# Command palette (Cmd+Shift+P): every shortcut in one clickable list.
. "$(dirname "$0")/../lib/common.sh"
pane=$HERDR_ACTIVE_PANE_ID
agent=$(herdr pane get "$pane" 2>/dev/null | jq -r '.result.pane.agent // empty')

menu() {
  # id<TAB>label<TAB>shortcut
  printf '%s\t%s\t%s\n' \
    ide        "󰨞  $T_pal_ide"        'Cmd+O' \
    finder     "  $T_pal_finder"     ''
  [ "$O_GIT_APP" != none ] && printf '%s\t%s\t%s\n' git "  $T_pal_git" 'Cmd+Shift+O'
  printf '%s\t%s\t%s\n' \
    changes    "✎  $T_pal_changes"    'Cmd+I' \
    find       "  $T_pal_find"       'Cmd+P' \
    grep       "  $T_pal_grep"       'Cmd+Shift+F' \
    browse     "  $T_pal_browse"     'Cmd+Shift+E'
  [ -n "$O_HAS_ACCOUNTS" ] && printf '%s\t%s\t%s\n' quota "󰚩  $T_pal_quota" 'Cmd+U'
  command -v ccusage >/dev/null && printf '%s\t%s\t%s\n' tokens "󰧑  $T_pal_tokens" ''
  [ "$W_pomodoro" = 1 ] && printf '%s\t%s\t%s\n' \
    pomo       "🍅  $T_pal_pomo"       '' \
    pomo-stop  "⏹  $T_pal_pomo_stop"  ''
  printf '%s\t%s\t%s\n' \
    standup    "  $T_pal_standup"    '' \
    recap      "🌙  $T_pal_recap"      '' \
    split-r    "  $T_pal_split_r"    'Ctrl+B v' \
    split-d    "  $T_pal_split_d"    'Ctrl+B -' \
    zoom       "  $T_pal_zoom"       'Ctrl+B z' \
    new-tab    "  $T_pal_new_tab"    'Ctrl+B c' \
    rename-tab "󰑕  $T_pal_rename_tab" ''
  [ "$W_tab_names" = 1 ] && printf '%s\t%s\t%s\n' auto-tab "󰑓  $T_pal_auto_tab" ''
  printf '%s\t%s\t%s\n' setup "  $T_pal_setup" 'Cmd+,'
  if [ "$agent" = claude ]; then
    printf '%s\t%s\t%s\n' \
      pr       "  $T_pal_pr"       '' \
      branches "  $T_pal_branches" '' \
      task     "󰄬  $T_pal_task"     ''
  fi
}

choice=$(menu | awk -F'\t' '{ printf "%s\t%-46s %s\n", $1, $2, $3 }' |
  fzf --ansi --delimiter='\t' --with-nth=2 --no-sort --cycle \
      --prompt="  $T_pal_prompt › " --header="$T_pal_header" |
  cut -f1)

case $choice in
  ide|git|finder) "$S/open-in.sh" "$choice" ;;
  changes)  exec "$S/changes.sh" ;;
  find)     exec "$S/find-file.sh" ;;
  grep)     exec "$S/search-text.sh" ;;
  browse)   cd "${HERDR_ACTIVE_PANE_CWD:-$HOME}" && exec yazi ;;
  quota)    exec "$S/usage-picker.sh" ;;
  tokens)   exec "$S/usage-report.sh" ;;
  split-r)  herdr pane split "$pane" --direction right --cwd "${HERDR_ACTIVE_PANE_CWD:-$HOME}" >/dev/null ;;
  split-d)  herdr pane split "$pane" --direction down --cwd "${HERDR_ACTIVE_PANE_CWD:-$HOME}" >/dev/null ;;
  zoom)     herdr pane zoom "$pane" >/dev/null ;;
  new-tab)  herdr tab create >/dev/null ;;
  pomo)      "$S/pomodoro.sh" start 25 ;;
  pomo-stop) "$S/pomodoro.sh" stop ;;
  standup)   "$S/worklog-popup.sh" standup ;;
  recap)     "$S/worklog-popup.sh" recap ;;
  setup)     exec "$ORCHA_HOME/bin/orcha" ;;
  rename-tab)
    printf '\n  %s ' "$T_pal_rename_ask"
    read -r name
    [ -n "$name" ] && herdr tab rename "$HERDR_ACTIVE_TAB_ID" "$name" >/dev/null ;;
  auto-tab)
    # back to its number: the tab namer picks it up again within a few seconds
    num=$(herdr tab get "$HERDR_ACTIVE_TAB_ID" 2>/dev/null | jq -r '.result.tab.number // empty')
    [ -n "$num" ] && herdr tab rename "$HERDR_ACTIVE_TAB_ID" "$num" >/dev/null && "$S/tab-names.py" ;;
  pr)       herdr pane run "$pane" "/pr-link" >/dev/null ;;
  branches) herdr pane run "$pane" "/branches" >/dev/null ;;
  task)     herdr pane run "$pane" "/task" >/dev/null ;;
esac
