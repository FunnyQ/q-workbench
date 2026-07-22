#!/usr/bin/env zsh
# Pick an SSH target in a plugin popup, then connect in a dedicated tab.

export PATH="$PATH:/opt/homebrew/bin"

registry="${Q_SSH_REGISTRY:-${0:A:h}/ssh-target-registry.zsh}"
editor="${Q_SSH_EDITOR:-${0:A:h}/ssh-target-editor.zsh}"
session_script="${0:A:h}/ssh-session.zsh"

result=$("$registry" | fzf --no-sort --print-query --prompt='SSH> ' \
  --read0 --highlight-line --gap --gap-line --pointer='▌' \
  --delimiter=$'\t' --with-nth=1 --accept-nth=2 \
  --border --border-label-pos=bottom \
  --border-label=' enter: connect · ctrl-i: edit · ctrl-x: remove ' \
  --bind "ctrl-i:execute($editor {2})+reload($registry)" \
  --bind "ctrl-x:execute-silent($registry remove {2})+reload($registry)") || exit 0

query=$(print -r -- "$result" | head -n1)
selected=$(print -r -- "$result" | sed -n '2p')
target="${selected:-$query}"
[[ -n "$target" ]] || exit 0

workspace_args=()
[[ -n "$HERDR_WORKSPACE_ID" ]] && workspace_args=(--workspace "$HERDR_WORKSPACE_ID")
tab_json=$(herdr tab create "${workspace_args[@]}" --label "󰢩  $target" \
  --env Q_NO_BANNER=1 --no-focus 2>/dev/null) || exit 1
pane=$(print -r -- "$tab_json" | jq -r '.result.root_pane.pane_id // empty')
tab=$(print -r -- "$tab_json" | jq -r '.result.tab.tab_id // empty')
[[ -n "$pane" && -n "$tab" ]] || exit 1

herdr pane run "$pane" "${(q)session_script} ${(q)target} ${(q)tab}" >/dev/null 2>&1 || {
  herdr tab close "$tab" >/dev/null 2>&1
  exit 1
}
herdr tab focus "$tab" >/dev/null 2>&1 || {
  herdr tab close "$tab" >/dev/null 2>&1
  exit 1
}
