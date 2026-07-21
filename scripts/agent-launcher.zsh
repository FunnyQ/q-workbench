#!/usr/bin/env zsh
# herdr-native agent launcher: optional git worktree, a harness, a usage label,
# then it builds the tab layout (yazi + term) and launches.
# Usage: agent-launcher.zsh <pane_id> [tab_id] [fixed_usage] [wt_mode] [layout_mode]
#   <pane_id>     the agent pane; renamed to the chosen usage label.
#   [tab_id]      when given, the tab is renamed to the label too.
#   [fixed_usage] when set, skips the usage menu and uses this as the label
#                 (ccc's pinned "main" tab).
#   [wt_mode]     "worktree" (alt+shift+c) → prompt for a branch and start in a
#                 fresh git worktree; empty (alt+c) → no worktree step.
#   [layout_mode] "no-layout" → restart in place: skip the yazi/term split (they
#                 already exist). Used by restart-agent.zsh (alt+shift+r).
#
# All menus run at the pane's FULL width; yazi + term are split off LAST, so every
# menu stays centered (no mid-flow resize) and a chosen worktree drives the new
# panes' cwd directly via `herdr pane split --cwd`.

export PATH="/opt/homebrew/bin:$PATH"

pane_id="$1"
tab_id="$2"
fixed_usage="$3"   # when set, skip the usage menu and use this as the label
wt_mode="$4"       # "worktree" → prompt for a branch and start in a git worktree
layout_mode="$5"   # "no-layout" → skip the yazi/term split (in-place restart)

# `tput` can briefly report the old/full terminal size while a restarted pane
# is settling. Herdr's layout is the source of truth for this pane's viewport.
layout=$(herdr pane layout --pane "$pane_id" 2>/dev/null)
cols=$(print -r -- "$layout" | jq -r --arg pane "$pane_id" \
  'first(.result.layout.panes[] | select(.pane_id == $pane) | .rect.width) // empty')
lines=$(print -r -- "$layout" | jq -r --arg pane "$pane_id" \
  'first(.result.layout.panes[] | select(.pane_id == $pane) | .rect.height) // empty')

[[ "$cols" == <-> && "$cols" -gt 0 ]] || cols=${COLUMNS:-0}
[[ "$lines" == <-> && "$lines" -gt 0 ]] || lines=${LINES:-0}
(( cols > 0 )) || cols=$(tput cols)
(( lines > 0 )) || lines=$(tput lines)
padding=$(( (lines - 12) / 2 ))
(( padding < 0 )) && padding=0

item_width=16
pad=$(printf '%*s' $(( (cols - item_width) / 2 )) '')

# Draw the bordered banner directly at a calculated margin. Wrapping an
# ANSI-styled multiline banner in another gum call offsets its border lines.
render_banner() {
  local title="$1"
  local subtitle="$2"
  local banner_width=44
  (( banner_width > cols - 4 )) && banner_width=$(( cols - 4 ))
  local margin_left=$(( (cols - banner_width - 2) / 2 ))
  (( margin_left < 0 )) && margin_left=0
  local banner

  banner=$(gum style --border rounded --padding '1 3' --width "$banner_width" \
    --bold "$title" \
    '' \
    "$(gum style --foreground 240 "$subtitle")")

  while IFS= read -r line; do
    printf '%*s%s\n' "$margin_left" '' "$line"
  done <<< "$banner"
}

# ----- Menu 1: git worktree (worktree mode only — alt+shift+c) ---------------
# A worktree is just another working dir for the same repo, so this step is
# harness-agnostic. Doing it first lets the chosen worktree drive the working dir
# for all three panes (agent + yazi + term), not just the agent pane — real
# parallel-work isolation. The worktree-vs-normal choice IS the keybinding
# (alt+shift+c vs alt+c), so there's no yes/no prompt — go straight to naming
# the branch.
worktree_dir=""
branch=""
if [[ -n "$wt_mode" ]] && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  repo_root=$(git rev-parse --show-toplevel)
  repo_name=${repo_root:t}
  git worktree prune 2>/dev/null   # drop stale registrations (manually-deleted worktrees)
  # Offer only branches NOT already checked out in a worktree (incl. the main
  # one) — git forbids the same branch in two worktrees, so listing them would
  # only fail `git worktree add`. --no-strict below still lets you type a new one.
  used=$(git worktree list --porcelain | sed -n 's#^branch refs/heads/##p')
  branches=$(git for-each-ref --format='%(refname:short)' refs/heads)
  [[ -n "$used" ]] && branches=$(print -r -- "$branches" | grep -vxF -- "$used")

  clear
  printf '\n%.0s' {1..$padding}
  render_banner '  New Worktree' \
    'Filter a branch, name a new one, or leave empty for auto.'
  echo

  if [[ -n "$branches" ]]; then
    # --no-strict: return the typed text when it matches no existing branch,
    # so the same field both picks an existing branch and names a new one.
    branch=$(print -r -- "$branches" | gum filter --no-strict --height 12 \
      --placeholder "type to filter or name a new branch…") || branch="__cancel__"
  else
    branch=$(gum input --placeholder "new branch name (empty = auto)…" --width "$cols") \
      || branch="__cancel__"
  fi

  if [[ "$branch" != "__cancel__" ]]; then
    branch=${branch//[[:space:]]/}                        # git branches carry no spaces
    [[ -z "$branch" ]] && branch="wt-$(date +%s)"   # unix timestamp
    worktree_dir="${repo_root:h}/${repo_name}-wt/${branch//\//-}"

    if [[ -d "$worktree_dir" ]]; then
      :                                                    # reuse an existing worktree dir
    elif git show-ref --verify --quiet "refs/heads/$branch"; then
      git worktree add "$worktree_dir" "$branch" >/dev/null 2>&1
    else
      git worktree add -b "$branch" "$worktree_dir" >/dev/null 2>&1
    fi

    [[ -d "$worktree_dir" ]] || { worktree_dir=""; branch=""; }   # add failed → fall back
  else
    branch=""
  fi
fi

# ----- Menu 2: harness -------------------------------------------------------
clear
printf '\n%.0s' {1..$padding}
render_banner '󱚟  Launch Agent' 'Choose a harness.'
echo

harness=$(gum choose --height 8 --no-show-help --cursor "" --header "" \
  "${pad}󱗎  claude code" \
  "${pad}  codex" \
  "${pad}󱚟  opencode")
[[ -z "$harness" ]] && exit 0

# ----- Menu 3: usage (becomes the pane/tab label) ----------------------------
if [[ -n "$fixed_usage" ]]; then
  label="$fixed_usage"
else
clear
printf '\n%.0s' {1..$padding}
render_banner '  Usage' 'What is this pane for?'
echo

usage=$(gum choose --height 8 --no-show-help --cursor "" --header "" \
  "${pad}  discuss" \
  "${pad}  review" \
  "${pad}  debug" \
  "${pad}󱦹  let me write…")
[[ -z "$usage" ]] && exit 0

label="${usage#"${usage%%[! ]*}"}"   # strip the leading pad
if [[ "$usage" == *"let me write"* ]]; then
  clear
  printf '\n%.0s' {1..$padding}
  label=$(gum input --placeholder "label for this pane…" --width "$cols")
  [[ -z "$label" ]] && exit 0
fi
fi

# ----- Apply names -----------------------------------------------------------
# Tag the label with the branch so parallel worktree tabs stay distinguishable.
[[ -n "$branch" ]] && label="$label  ${branch}"
[[ -n "$pane_id" ]] && herdr pane rename "$pane_id" "$label" >/dev/null 2>&1
[[ -n "$tab_id"  ]] && herdr tab  rename "$tab_id"  "$label" >/dev/null 2>&1

if [[ -n "$worktree_dir" ]]; then
  project_dir="$worktree_dir"
else
  project_dir=$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")
fi
# ----- Resolve the launch command (menus still run at full width) ------------
# Build the command now; the exec + pane split happen at the very end, so every
# menu — including claude's model picker below — renders before any resize.
launch=()
if [[ "$harness" == *"codex"* ]]; then
  launch=(codex --dangerously-bypass-approvals-and-sandbox)
elif [[ "$harness" == *"opencode"* ]]; then
  launch=(opencode)
else
# claude code → model selection
clear
printf '\n%.0s' {1..$padding}
render_banner '󰧑 claude code' 'Choose a model.'
echo

mpad=$(printf '%*s' $(( (cols - 24) / 2 )) '')
selection=$(gum choose --height 6 --no-show-help --cursor "" --header "" \
  "${mpad}Opus" \
  "${mpad}OpusPlan (Sonnet)" \
  "${mpad}CCR" \
  "${mpad}Fable 5")
[[ -z "$selection" ]] && exit 0
label="${selection#"${selection%%[! ]*}"}"  # strip leading pad

effort_args=()
case "$label" in
  "Opus")             model="claude-opus-4-8" ;;
  "OpusPlan (Sonnet)") model="opusplan"; effort_args=(--effort medium) ;;
  "CCR")              model="CCR" ;;
  "Fable 5")          model="claude-fable-5" ;;
esac

if [[ "$model" == "CCR" ]]; then
  launch=(ccr code)
else
  launch=(claude --model "$model" "${effort_args[@]}" --dangerously-load-development-channels plugin:monitor@q-lab-marketplace)
fi
fi   # end harness → launch dispatch

# ----- Build the tab layout LAST: yazi + term, born in $project_dir ----------
# Deferred until every menu is done, so all menus ran at full width (stable
# centering) and a chosen worktree drives the new panes' cwd directly via --cwd.
# Skipped on an in-place restart (no-layout): the yazi/term panes already exist.
if [[ "$layout_mode" != "no-layout" ]]; then
  split_json=$(herdr pane split "$pane_id" --direction right --ratio 0.38 --cwd "$project_dir" --env Q_NO_BANNER=1 --no-focus 2>/dev/null)
  yazi_pane=$(print -r -- "$split_json" | jq -r '.result.pane.pane_id')
  if [[ -n "$yazi_pane" && "$yazi_pane" != "null" ]]; then
    herdr pane rename "$yazi_pane" "󰥨  Files" >/dev/null 2>&1
    herdr pane run "$yazi_pane" "yazi ." >/dev/null 2>&1
    term_json=$(herdr pane split "$yazi_pane" --direction down --ratio 0.9 --cwd "$project_dir" --no-focus 2>/dev/null)
    term_pane=$(print -r -- "$term_json" | jq -r '.result.pane.pane_id')
    [[ -n "$term_pane" && "$term_pane" != "null" ]] && herdr pane rename "$term_pane" "  term" >/dev/null 2>&1
  fi
fi

cd "$project_dir"
clear
exec "${launch[@]}"
