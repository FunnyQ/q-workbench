#!/usr/bin/env zsh

export PATH="/opt/homebrew/bin:$PATH"

SCRIPT_DIR="${0:A:h}"

popup_cols=${COLUMNS:-0}
[[ "$popup_cols" == <-> && "$popup_cols" -gt 0 ]] || popup_cols=$(tput cols)
content_width=44
(( content_width > popup_cols - 4 )) && content_width=$(( popup_cols - 4 ))
content_margin=$(( (popup_cols - content_width - 2) / 2 ))
(( content_margin < 0 )) && content_margin=0

banner=$(gum style --border rounded --padding '1 3' --width "$content_width" \
  --bold '󰀪  Current session will end' '' \
  "$(gum style --foreground 240 'The agent will relaunch in place.')")

gum confirm \
  --affirmative "Restart" \
  --negative "Cancel" \
  --selected.background 214 \
  --selected.foreground 235 \
  --unselected.background 237 \
  --unselected.foreground 223 \
  --padding "1 $content_margin" \
  "$banner" || exit 0

if [[ -n "$HERDR_ACTIVE_PANE_ID" ]]; then
  export HERDR_PLUGIN_CONTEXT_JSON=$(jq -nc \
    --arg pane "$HERDR_ACTIVE_PANE_ID" \
    '{focused_pane_id: $pane}')
fi

exec zsh "$SCRIPT_DIR/restart-agent.zsh"
