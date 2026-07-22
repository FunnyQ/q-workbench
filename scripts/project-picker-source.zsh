#!/usr/bin/env zsh
# Emit registered projects plus the best zoxide match for an active query.

set -eu

export PATH="$PATH:/opt/homebrew/bin"

registry="${Q_PROJECT_REGISTRY_FILE:-$HOME/.local/state/herdr-projects/registry.json}"
query="${1:-}"

[[ -f "$registry" ]] || exit 1

jq -j --arg home "$HOME" '
  .projects | to_entries |
  map(select(.value.hidden != true)) |
  sort_by(if .value.last_used_at then [0, -(.value.last_used_at)] else [1, .value.name] end)[] |
  (.key | if startswith($home + "/") then "~/" + ltrimstr($home + "/") else . end) as $path |
  ((.value.aliases // []) | join(" | ")) as $aliases |
  "󰉋  \(.value.name)" +
  (if $aliases == "" then "" else " | \($aliases)" end) +
  "\n   \($path)\n   \((.value.sources // []) | join(" · "))" +
  "\t\(.key)\u0000"
' "$registry"

(( ${#query} >= 2 )) || exit 0
command -v zoxide >/dev/null || exit 0
zoxide_path=$(zoxide query -- "$query" 2>/dev/null) || exit 0
[[ -d "$zoxide_path" ]] || exit 0
zoxide_path=$(cd -P -- "$zoxide_path" && pwd)

jq -e --arg project "$zoxide_path" '.projects[$project] != null' "$registry" \
  >/dev/null 2>&1 && exit 0

display_path="$zoxide_path"
[[ "$zoxide_path" == "$HOME"/* ]] && display_path="~/${zoxide_path#"$HOME"/}"
print -rn -- "󰉋  ${zoxide_path:t}"$'\n'"   $display_path"$'\n''   zoxide'$'\t'"$zoxide_path"$'\0'
