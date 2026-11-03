#!/bin/sh
# A tidy demo for screenshots: two sample projects (a tea shop, a payments API) with branches, task titles
# and uncommitted changes, opened in the herdr session you run this from.
# Real emails are hidden while ~/.config/orcha/demo exists.
#
#   herdr --session demo        (a separate session: your real projects stay out of the sidebar)
#   ~/.orcha/docs/demo.sh       in its first pane
#   ~/.orcha/docs/demo.sh off   afterwards: real emails back, demo folder removed
set -e
D="$HOME/orcha-demo"
CFG="${XDG_CONFIG_HOME:-$HOME/.config}/orcha"
CACHE="$HOME/.cache/orcha/tracker"

if [ "$1" = off ]; then
  rm -f "$CFG/demo"
  rm -rf "$D" "$CACHE"/github_orcha_demo_*
  echo "demo off"
  exit 0
fi

touch "$CFG/demo"
rm -rf "$D"; mkdir -p "$D" "$CACHE"

repo() {  # <name> <origin> <branch> <task id> <title> <state>
  r="$D/$1"; mkdir -p "$r/src"; cd "$r"
  git init -q -b main
  git config user.name "Demo"; git config user.email "you@example.com"
  git remote add origin "$2"
  printf 'export const theme = "light"\n' > src/theme.ts
  printf '# %s\n' "$1" > README.md
  git add -A; git commit -q -m "first commit"
  git checkout -q -b "$3"
  printf 'export const theme = "dark"\nexport const accent = "#ffc4dc"\n' > src/theme.ts
  git commit -qam "dark theme tokens"
  # the cache file tracker.py looks for: github_<owner>_<repo>-<id>.json
  key=$(printf 'github_%s' "$(printf '%s' "$2" | sed -E 's#.*github.com[:/]##; s#\.git$##; s#[^A-Za-z0-9]+#_#g')")
  printf '{"id": "%s", "title": "%s", "state": "%s", "type": "Issue", "assignee": null, "summary": "", "url": ""}\n' \
    "$4" "$5" "$6" > "$CACHE/$key-$4.json"
  touch -t 203001010000 "$CACHE/$key-$4.json"   # stays "fresh" while the demo runs
}

repo matcha-house https://github.com/orcha-demo/matcha-house.git feature/42-dark-menu 42 "Dark mode for the menu" Open
# work in progress: what Cmd+I shows
printf 'import { theme } from "./theme"\n\nexport function Menu() {\n  return <Toggle label="Dark mode" value={theme === "dark"} />\n}\n' > "$D/matcha-house/src/Menu.tsx"
printf 'export const theme = "dark"\nexport const accent = "#ffc4dc"\nexport const radius = 12\n' > "$D/matcha-house/src/theme.ts"
repo orca-pay https://github.com/orcha-demo/orca-pay.git feature/108-apple-pay 108 "Apple Pay at checkout" "In progress"
printf 'export const APPLE_PAY = { merchant: "merchant.orca.pay", networks: ["visa", "mastercard"] }\n' > "$D/orca-pay/src/applePay.ts"
printf 'export const theme = "dark"\nexport const currency = "USD"\n' > "$D/orca-pay/src/theme.ts"

ws() {  # <dir> <label>: a workspace, its first pane's id printed
  w=$(herdr workspace create --cwd "$1" --label "$2" --focus | jq -r '.. | .workspace_id? // empty' | head -1)
  sleep 0.5
  herdr pane list | jq -r --arg w "$w" '.result.panes[] | select(.pane_id | startswith($w + ":")) | .pane_id' | head -1
}
p1=$(ws "$D/matcha-house" matcha-house)
[ -n "$p1" ] && herdr pane run "$p1" "clear; claude" >/dev/null
p2=$(herdr pane split "$p1" --direction right --cwd "$D/matcha-house" | jq -r ".result.pane.pane_id")
# with a prompt: plain `codex` opens its task list, which shows your real projects
herdr pane run "$p2" 'clear; codex "Explain src/theme.ts in two sentences."' >/dev/null
p3=$(ws "$D/orca-pay" orca-pay)
[ -n "$p3" ] && herdr pane run "$p3" "clear; claude" >/dev/null
echo "demo ready: ask each agent something small so they show as working"
