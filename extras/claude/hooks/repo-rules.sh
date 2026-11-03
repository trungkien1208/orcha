# shellcheck shell=sh
# repo_rules <repo dir>: the repo rules as JSON. config.json's "repo" section,
# with the repo's own .orcha.json on top (a team can commit one):
#   { "protected": "^(main|develop)$", "check": "npm run typecheck",
#     "block": ["prettier .*--write", {"pattern": "rm -rf /", "reason": "no"}] }
repo_rules() {
  global=$(jq -c '.repo // {}' "$ORCHA_CONFIG_DIR/config.json" 2>/dev/null || echo '{}')
  local_rules=$(jq -c '.' "$1/.orcha.json" 2>/dev/null || echo '{}')
  jq -cn --argjson g "$global" --argjson l "$local_rules" \
    '{protected: "^(release/.+|develop|main|master)$", check: "auto", block: []} + $g + $l'
}
