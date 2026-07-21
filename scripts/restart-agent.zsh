#!/usr/bin/env zsh
# Restart the agent running in the focused (or same-tab) agent pane, IN PLACE —
# without tearing down the yazi/term side panes. Runs as a plugin action, outside
# the pane's process group, so it survives the agent it kills.
#
# Why it works: build-agent-tab runs the launcher via `herdr pane run` (types the
# command into the pane's interactive shell), so agent-launcher's `exec <agent>`
# replaces the *launcher* subprocess, not the pane's shell. Killing the agent's
# foreground process group therefore drops the pane back to its shell prompt —
# the pane lives on — and we re-inject the launcher in no-layout mode, which
# re-runs the harness/model menus and execs a fresh agent (skipping the split,
# and reusing the current label so the usage menu is skipped too).

export PATH="/opt/homebrew/bin:$PATH"

LAUNCHER="${0:A:h}/agent-launcher.zsh"

# --- Resolve the target agent pane --------------------------------------------
invocation_pane=$(print -r -- "$HERDR_PLUGIN_CONTEXT_JSON" | \
  jq -r '.focused_pane_id // empty' 2>/dev/null)
if [[ -n "$invocation_pane" ]]; then
  cur=$(herdr pane current --pane "$invocation_pane" 2>/dev/null) || exit 1
else
  cur=$(herdr pane current 2>/dev/null) || exit 1
fi
cur_pane=$(print -r -- "$cur"  | jq -r '.result.pane.pane_id')
cur_tab=$(print -r -- "$cur"   | jq -r '.result.pane.tab_id')
cur_agent=$(print -r -- "$cur" | jq -r '.result.pane.agent // ""')
label=$(print -r -- "$cur"     | jq -r '.result.pane.label // ""')

if [[ -n "$cur_agent" && "$cur_agent" != "null" ]]; then
  target="$cur_pane"
else
  # Focus is on a non-agent pane (yazi/term) — restart the agent in the same tab.
  panes=$(herdr pane list 2>/dev/null) || exit 1
  match=$(print -r -- "$panes" | jq -r --arg tab "$cur_tab" \
    'first(.result.panes[] | select(.tab_id == $tab and .agent != null)) | "\(.pane_id)\t\(.label)"')
  if [[ -z "$match" ]]; then
    herdr notification show "Restart agent" \
      --body "No agent pane in this tab to restart." --position bottom-right >/dev/null 2>&1
    exit 0
  fi
  IFS=$'\t' read -r target label <<<"$match"
fi
[[ "$label" == "null" ]] && label=""

# A plugin action does not move keyboard focus. When alt+r is invoked
# from yazi/term, focus the adjacent agent pane before opening its menus.
if [[ "$cur_pane" != "$target" ]]; then
  focus_direction=""
  for direction in left right up down; do
    neighbor=$(herdr pane neighbor --direction "$direction" --pane "$cur_pane" 2>/dev/null)
    neighbor_pane=$(print -r -- "$neighbor" | jq -r '.result.neighbor.neighbor_pane_id // empty')
    if [[ "$neighbor_pane" == "$target" ]]; then
      focus_direction="$direction"
      break
    fi
  done

  if [[ -z "$focus_direction" ]]; then
    herdr notification show "Restart agent" \
      --body "Could not focus the agent pane." --position bottom-right >/dev/null 2>&1
    exit 1
  fi

  herdr pane focus --direction "$focus_direction" --pane "$cur_pane" >/dev/null 2>&1 || exit 1
fi

# --- Kill the agent's foreground process group (the pane's shell survives) -----
info=$(herdr pane process-info --pane "$target" 2>/dev/null)
pgid=$(print -r -- "$info"      | jq -r '.result.process_info.foreground_process_group_id // empty')
shell_pid=$(print -r -- "$info" | jq -r '.result.process_info.shell_pid // empty')

if [[ -n "$pgid" && "$pgid" != "0" && "$pgid" != "$shell_pid" ]]; then
  kill -TERM -"$pgid" 2>/dev/null
  for _ in {1..50}; do kill -0 "$pgid" 2>/dev/null || break; sleep 0.1; done
  kill -0 "$pgid" 2>/dev/null && kill -KILL -"$pgid" 2>/dev/null
  sleep 0.3   # let the shell settle back to its prompt before injecting
fi

# --- Restore the TTY and re-inject the launcher in no-layout mode -------------
# Codex can leave the pane in raw mode and the Kitty keyboard protocol enabled
# when its process group is terminated. Disabled ONLCR makes each newline
# continue at the old column; Kitty CSI-u sequences make Gum ignore arrow keys.
# Run the reset inside the pane so it owns the target TTY; this detached script
# has no access to it.
# Args: <pane> <tab_id> <fixed_usage> <wt_mode> <layout_mode>
# Empty tab_id (don't rename the tab) + fixed_usage=current label (skip the usage
# menu, keep the label) + empty wt_mode (no worktree step) + no-layout.
herdr pane run "$target" \
  "stty sane; printf '\\033[<u\\033[?7h\\033[?25h\\033[0m'; $LAUNCHER '$target' '' '$label' '' no-layout"
