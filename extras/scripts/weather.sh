#!/bin/sh
# Weather for the header: "⛅ 31°", plus "☔" when rain is around.
# From wttr.in (your city from config, else located by IP), cached for 30
# minutes; prints nothing offline.
. "$(dirname "$0")/../lib/common.sh"
C="$ORCHA_CACHE/weather"
if [ ! -s "$C" ] || [ -n "$(find "$C" -mmin +30 2>/dev/null)" ]; then
  city=$(jq -rn --arg c "$O_WEATHER_CITY" '$c | @uri')
  # %c icon | %t temperature | %C condition text
  w=$(curl -s -m 4 "https://wttr.in/$city?format=%c|%t|%C" 2>/dev/null)
  case $w in *'|'*'|'*) printf '%s\n' "$w" > "$C" ;; esac
fi
[ -s "$C" ] || exit 0
IFS='|' read -r icon temp cond < "$C"
icon=$(printf '%s' "$icon" | tr -d ' \357\270\217')   # drop spaces and emoji variation selectors
temp=$(printf '%s' "$temp" | sed 's/^+//; s/[CF]$//')
rain=""
printf '%s' "$cond" | grep -qiE 'rain|shower|drizzle|thunder' && rain=" ☔"
printf '%s %s%s\n' "$icon" "$temp" "$rain"
