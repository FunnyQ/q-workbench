#!/usr/bin/env zsh
# Pick a registered project, then focus or create its Herdr workspace.

set -eu

export PATH="$PATH:/opt/homebrew/bin"

source "${0:A:h}/config.zsh"

registry="$Q_PROJECT_REGISTRY_FILE"
registry_script="${0:A:h}/project-registry.zsh"
tab_builder="${0:A:h}/build-agent-tab.zsh"
source_script="${0:A:h}/project-picker-source.zsh"

[[ -f "$registry" ]] || {
  print -u2 "project picker: registry not found: $registry"
  exit 1
}

result=$("$source_script" | fzf --read0 --print-query --expect=alt-enter --prompt='Project> ' \
  --highlight-line --pointer='▌' --info=inline-right \
  --delimiter=$'\t' --with-nth=1 --accept-nth=2 \
  --bind "change:reload(${(q)source_script} {q})" \
  --bind "ctrl-i:execute(${(q)registry_script} edit {2})+reload(${(q)source_script} {q})" \
  --border --border-label-pos=bottom \
  --border-label=' enter: agent · alt-enter: plain · ctrl-i: edit · typing searches zoxide ') || exit 0

query=$(print -r -- "$result" | head -n1)
key=$(print -r -- "$result" | sed -n '2p')
selected=$(print -r -- "$result" | sed -n '3p')
project_path="$selected"

if [[ -z "$project_path" && -n "$query" ]]; then
  if [[ -d "$query" ]]; then
    project_path="$query"
  elif command -v zoxide >/dev/null; then
    project_path=$(zoxide query -- "$query" 2>/dev/null) || true
  fi
fi

[[ -n "$project_path" && -d "$project_path" ]] || {
  print -u2 -- "project picker: project not found: ${query:-$project_path}"
  exit 1
}

project_path=$(cd -P -- "$project_path" && pwd)
label=$(jq -r --arg project "$project_path" \
  '.projects[$project].name // ($project | split("/") | last)' "$registry")

snapshot=$(herdr api snapshot 2>/dev/null) || exit 1
workspace_id=$(jq -r --arg project "$project_path" '
  [.result.snapshot.panes[]? |
    select(.cwd == $project or .foreground_cwd == $project) |
    .workspace_id][0] // empty
' <<< "$snapshot")

if [[ -n "$workspace_id" ]]; then
  herdr workspace focus "$workspace_id" >/dev/null || exit 1
else
  workspace_json=$(herdr workspace create --cwd "$project_path" --label "$label" \
    --env Q_NO_BANNER=1 --no-focus 2>/dev/null) || exit 1
  workspace_id=$(jq -r '.result.workspace.workspace_id // empty' <<< "$workspace_json")
  [[ -n "$workspace_id" ]] || exit 1

  if [[ "$key" != alt-enter ]]; then
    agent_pane=$(jq -r '.result.root_pane.pane_id // empty' <<< "$workspace_json")
    tab_id=$(jq -r '.result.tab.tab_id // empty' <<< "$workspace_json")
    [[ -n "$agent_pane" && -n "$tab_id" ]] || exit 1
    herdr tab rename "$tab_id" '󰧑  main' >/dev/null 2>&1 || exit 1
    "$tab_builder" "$agent_pane" '' '󰧑  main' || exit 1
  fi
  herdr workspace focus "$workspace_id" >/dev/null || exit 1
fi

"$registry_script" use "$project_path"
