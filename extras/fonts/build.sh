#!/bin/sh
# Build the "Agent Icons" font (Claude / OpenAI / Cursor logos at U+100000..2,
# the orcha whale at U+100003)
# and install it into ~/Library/Fonts. The logos are their owners' trademarks,
# so orcha ships no copy: they are fetched from the simple-icons and lobehub
# icon packages (pinned versions) and the font is made on your own machine.
set -e
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/logos"
get() { curl -fsSL -m 20 -o "$work/logos/$1" "$2"; }
get claude.svg https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/claude.svg
get openai.svg https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/openai.svg
get cursor.svg https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@1.73.0/icons/cursor.svg
cp "$here/orcha.svg" "$work/logos/orcha.svg"
# fonttools + skia-pathops in a throwaway venv, so nothing lands in your Python
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install -q fonttools==4.55.3 skia-pathops==0.8.0.post2
"$work/venv/bin/python" -I "$here/build_agent_icons.py" "$work/logos" "$work/AgentIcons.ttf"
mkdir -p "$HOME/Library/Fonts"
cp "$work/AgentIcons.ttf" "$HOME/Library/Fonts/AgentIcons.ttf"
echo "installed ~/Library/Fonts/AgentIcons.ttf"
