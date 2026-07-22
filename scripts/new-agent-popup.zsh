#!/usr/bin/env zsh
# Select an agent in a modal popup, build its tab in the background, then focus it.

export PATH="$PATH:/opt/homebrew/bin"

wt_mode="$1"
popup_cols=${COLUMNS:-0}
popup_lines=${LINES:-0}
[[ "$popup_cols" == <-> && "$popup_cols" -gt 0 ]] || popup_cols=$(tput cols)
[[ "$popup_lines" == <-> && "$popup_lines" -gt 0 ]] || popup_lines=$(tput lines)
content_width=44
(( content_width > popup_cols - 4 )) && content_width=$(( popup_cols - 4 ))
content_margin=$(( (popup_cols - content_width - 2) / 2 ))
choice_margin=$(( (popup_cols - 24) / 2 ))
vertical_padding=$(( (popup_lines - 14) / 2 ))
(( content_margin < 0 )) && content_margin=0
(( choice_margin < 0 )) && choice_margin=0
(( vertical_padding < 0 )) && vertical_padding=0
choice_pad=$(printf '%*s' "$choice_margin" '')

render_menu() {
  local title="$1"
  local subtitle="$2"
  local banner

  [[ -t 1 ]] && clear
  printf '\n%.0s' {1..$vertical_padding}
  banner=$(gum style --border rounded --padding '1 3' --width "$content_width" \
    --bold "$title" '' "$(gum style --foreground 240 "$subtitle")")
  while IFS= read -r line; do
    printf '%*s%s\n' "$content_margin" '' "$line"
  done <<< "$banner"
  print
}

# Worktree mode selects or creates the branch before any Herdr resources exist.
project_dir=''
branch=''
if [[ "$wt_mode" == 'worktree' ]] && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  repo_root=$(git rev-parse --show-toplevel)
  repo_name=${repo_root:t}
  git worktree prune 2>/dev/null
  used=$(git worktree list --porcelain | sed -n 's#^branch refs/heads/##p')
  branches=$(git for-each-ref --format='%(refname:short)' refs/heads)

  if [[ -n "$used" ]]; then
    available=''
    while IFS= read -r candidate; do
      [[ -n "$candidate" ]] || continue
      if ! print -r -- "$used" | rg -Fxq -- "$candidate"; then
        available+="${choice_pad}${candidate}"$'\n'
      fi
    done <<< "$branches"
    branches=${available%$'\n'}
  else
    branches=$(print -r -- "$branches" | sed "s/^/${choice_pad}/")
  fi

  render_menu '  New Worktree' 'Filter a branch, or name a new one.'
  if [[ -n "$branches" ]]; then
    branch=$(print -r -- "$branches" | gum filter --no-strict --height 12 \
      --placeholder 'filter or name a branch…') || exit 0
  else
    printf '%*s' "$choice_margin" ''
    branch=$(gum input --placeholder 'new branch name…' --width 44) || exit 0
  fi
  branch="${branch#"${branch%%[![:space:]]*}"}"
  branch=${branch//[[:space:]]/}
  [[ -n "$branch" ]] || branch="wt-$(date +%s)"
  project_dir="${repo_root:h}/${repo_name}-wt/${branch//\//-}"

  if [[ -d "$project_dir" ]]; then
    :
  elif git show-ref --verify --quiet "refs/heads/$branch"; then
    git worktree add "$project_dir" "$branch" >/dev/null 2>&1 || exit 1
  else
    git worktree add -b "$branch" "$project_dir" >/dev/null 2>&1 || exit 1
  fi
fi

render_menu '󱚟  Launch Agent' 'Choose a harness.'
harness=$(gum choose --height 8 --no-show-help --cursor '' --header '' \
  "${choice_pad}󱗎  claude code" \
  "${choice_pad}  codex" \
  "${choice_pad}󱚟  opencode") || exit 0
[[ -n "$harness" ]] || exit 0
harness="${harness#"${harness%%[![:space:]]*}"}"

model=''
if [[ "$harness" == *'claude code'* ]]; then
  render_menu '󰧑  claude code' 'Choose a model.'
  model_choice=$(gum choose --height 6 --no-show-help --cursor '' --header '' \
    "${choice_pad}Opus" \
    "${choice_pad}OpusPlan (Sonnet)" \
    "${choice_pad}CCR" \
    "${choice_pad}Fable 5") || exit 0
  [[ -n "$model_choice" ]] || exit 0
  model_choice="${model_choice#"${model_choice%%[![:space:]]*}"}"

  case "$model_choice" in
    'Opus') model='claude-opus-4-8' ;;
    'OpusPlan (Sonnet)') model='opusplan' ;;
    'CCR') model='CCR' ;;
    'Fable 5') model='claude-fable-5' ;;
  esac
fi

render_menu '  Usage' 'What is this tab for?'
usage=$(gum choose --height 8 --no-show-help --cursor '' --header '' \
  "${choice_pad}  discuss" \
  "${choice_pad}  review" \
  "${choice_pad}  debug" \
  "${choice_pad}󱦹  let me write…") || exit 0
[[ -n "$usage" ]] || exit 0
usage="${usage#"${usage%%[![:space:]]*}"}"

case "$usage" in
  *'let me write'*)
    render_menu '  Usage' 'Name this tab.'
    label=$(gum input --placeholder 'label for this tab…' --width 40) || exit 0
    ;;
  *) label="$usage" ;;   # already pad-stripped above; keep the nerd font icon
esac
[[ -n "$label" ]] || exit 0

[[ -n "$project_dir" ]] || project_dir=$(git rev-parse --show-toplevel 2>/dev/null || print -r -- "$PWD")
[[ -n "$branch" ]] && label="$label  $branch"

workspace_args=()
[[ -n "$HERDR_WORKSPACE_ID" ]] && workspace_args=(--workspace "$HERDR_WORKSPACE_ID")
tab_json=$(herdr tab create "${workspace_args[@]}" --label "$label" --cwd "$project_dir" \
  --env Q_NO_BANNER=1 --no-focus 2>/dev/null) || exit 1
agent_pane=$(print -r -- "$tab_json" | jq -r '.result.root_pane.pane_id // empty')
tab_id=$(print -r -- "$tab_json" | jq -r '.result.tab.tab_id // empty')
[[ -n "$agent_pane" && -n "$tab_id" ]] || exit 1

cleanup_tab() {
  herdr tab close "$tab_id" >/dev/null 2>&1
  herdr notification show 'Agent tab failed' --body 'The incomplete tab was closed.' \
    --sound none >/dev/null 2>&1
  exit 1
}

herdr pane rename "$agent_pane" "$label" >/dev/null 2>&1 || cleanup_tab
herdr tab rename "$tab_id" "$label" >/dev/null 2>&1 || cleanup_tab

split_json=$(herdr pane split "$agent_pane" --direction right --ratio 0.38 \
  --cwd "$project_dir" --env Q_NO_BANNER=1 --no-focus 2>/dev/null) || cleanup_tab
yazi_pane=$(print -r -- "$split_json" | jq -r '.result.pane.pane_id // empty')
[[ -n "$yazi_pane" ]] || cleanup_tab

herdr pane rename "$yazi_pane" '󰥨  Files' >/dev/null 2>&1 || cleanup_tab
herdr pane run "$yazi_pane" 'yazi .' >/dev/null 2>&1 || cleanup_tab

term_json=$(herdr pane split "$yazi_pane" --direction down --ratio 0.9 \
  --cwd "$project_dir" --no-focus 2>/dev/null) || cleanup_tab
term_pane=$(print -r -- "$term_json" | jq -r '.result.pane.pane_id // empty')
[[ -n "$term_pane" ]] || cleanup_tab
herdr pane rename "$term_pane" '  term' >/dev/null 2>&1 || cleanup_tab

case "$harness" in
  *codex*) launch='codex --dangerously-bypass-approvals-and-sandbox' ;;
  *opencode*) launch='opencode' ;;
  *)
    if [[ "$model" == 'CCR' ]]; then
      launch='ccr code'
    elif [[ "$model" == 'opusplan' ]]; then
      launch="claude --model opusplan --effort medium --dangerously-load-development-channels plugin:monitor@q-lab-marketplace"
    else
      launch="claude --model $model --dangerously-load-development-channels plugin:monitor@q-lab-marketplace"
    fi
    ;;
esac

herdr pane run "$agent_pane" "$launch" >/dev/null 2>&1 || cleanup_tab
herdr tab focus "$tab_id" >/dev/null 2>&1 || cleanup_tab
