#!/usr/bin/env zsh
# Create a dedicated dashboard launcher tab, start Claude Code with Sonnet,
# and submit the dashboard/cockpit prompt as the initial Claude prompt.

# Ensure herdr, jq, and claude are reachable under a minimal detached PATH.
export PATH="$HOME/.bun/bin:/opt/homebrew/bin:$PATH"

readonly tab_label="  Dashboard Launcher"
readonly prompt="/usage-dashboard and restart /cockpit server"
readonly workspace_label="personal-assistant"

# Resolve the workspace every time because herdr workspace IDs are not durable.
workspaces_json=$(herdr workspace list 2>/dev/null) || exit 1
workspace_id=$(print -r -- "$workspaces_json" | jq -r --arg label "$workspace_label" \
  'first(.result.workspaces[] | select(.label == $label)) | .workspace_id // empty')
if [[ -z "$workspace_id" ]]; then
  herdr notification show "Dashboard Launcher" \
    --body "Workspace '$workspace_label' was not found." \
    --position bottom-right >/dev/null 2>&1
  exit 1
fi

tab_json=$(herdr tab create --workspace "$workspace_id" --label "$tab_label" \
  --env Q_NO_BANNER=1 --focus 2>/dev/null) || exit 1
pane=$(print -r -- "$tab_json" | jq -r '.result.root_pane.pane_id')
[[ -n "$pane" && "$pane" != "null" ]] || exit 1

# pane run submits the command atomically. Passing the prompt to `claude`
# makes Claude start processing it immediately instead of leaving it staged.
herdr pane run "$pane" "claude --model sonnet ${(q)prompt}"
