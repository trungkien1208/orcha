#!/bin/sh
# Open the focused pane's project (its git root, else its folder) in an app.
#   open-in.sh ide | git | finder
. "$(dirname "$0")/../lib/common.sh"
dir=${HERDR_ACTIVE_PANE_CWD:-$PWD}
root=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || root=$dir
case $1 in
  ide)
    case $O_IDE in
      nvim) HERDR_ACTIVE_PANE_CWD=$root in_pane "nvim ." ;;
      *)    open -a "$O_IDE_APP" "$root" ;;
    esac ;;
  git)
    case $O_GIT_APP in
      lazygit) HERDR_ACTIVE_PANE_CWD=$root in_pane "lazygit" ;;
      *)       open -a "$O_GIT_APP_NAME" "$root" ;;
    esac ;;
  finder) open "$root" ;;
  *) echo "usage: open-in.sh ide|git|finder" >&2; exit 1 ;;
esac
