#!/usr/bin/env zsh
# Run the herdr agent launcher inside an existing tab's pane.
# Used by ccc (zsh/functions/herdr.zsh).
#
# The launcher owns the whole in-pane flow: it shows the worktree + harness +
# usage menus at the pane's full width, and only THEN splits the yazi + term
# panes (born in the chosen worktree via `herdr pane split --cwd`). Keeping the
# split out of here means no pane resizes mid-menu, so the centered gum menus
# stay put — and a chosen worktree drives every pane's cwd, not just the agent's.
#
# Usage: build-agent-tab.zsh <root_pane_id> [tab_id] [fixed_usage] [wt_mode]
#   <root_pane_id> → the agent pane the launcher runs in
#   [tab_id]       → forwarded to the launcher so it renames the TAB on selection
#                    (omit for ccc's first tab so the project tab name is kept)
#   [fixed_usage]  → forwarded to the launcher; when set it skips the usage menu
#                    and uses this label (ccc's first tab pins "main")
#   [wt_mode]      → forwarded to the launcher; "worktree" starts in a git worktree
export PATH="/opt/homebrew/bin:$PATH"

agent_pane="$1"
tab_id="$2"
fixed_usage="$3"
wt_mode="$4"        # "worktree" → launcher prompts for a branch + starts a worktree
[[ -n "$agent_pane" ]] || exit 1
launcher="${0:A:h}/agent-launcher.zsh"

herdr pane rename "$agent_pane" "󱚟  agent" >/dev/null 2>&1
# Quote each arg so an empty tab_id/fixed_usage keeps its slot (args don't shift).
herdr pane run "$agent_pane" "$launcher '$agent_pane' '$tab_id' '$fixed_usage' '$wt_mode'"
