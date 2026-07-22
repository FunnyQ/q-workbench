#!/usr/bin/env zsh
# Shared defaults for the workbench launchers. Sourced, never executed — keep it
# side-effect free (no output, no `set` changes, no exits).
#
# Precedence: the user config > environment > the defaults below. The user file is
# sourced first so its plain assignments win the `:-` fallbacks.
#
# It lives in Herdr's per-plugin config dir (`herdr plugin config-dir q.workbench`),
# not in this repo: machine-specific values then survive a reinstall and can never
# be committed by accident. The literal path is a fallback for when the CLI is
# unavailable — tests point Q_WORKBENCH_LOCAL_CONFIG at /dev/null to opt out.
#
# Herdr runs plugin actions detached, so a value exported from ~/.zshrc may not
# reach these scripts. This file is the reliable channel.

if [[ -z "${Q_WORKBENCH_LOCAL_CONFIG:-}" ]]; then
  Q_WORKBENCH_CONFIG_DIR="${Q_WORKBENCH_CONFIG_DIR:-$(herdr plugin config-dir q.workbench 2>/dev/null)}"
  : "${Q_WORKBENCH_CONFIG_DIR:=${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench}"
  Q_WORKBENCH_LOCAL_CONFIG="$Q_WORKBENCH_CONFIG_DIR/config.zsh"
fi
[[ -r "$Q_WORKBENCH_LOCAL_CONFIG" ]] && source "$Q_WORKBENCH_LOCAL_CONFIG"

# Workspace the dashboard launcher opens its tab in. The launcher aborts with a
# notification when no workspace carries this label.
Q_DASHBOARD_WORKSPACE="${Q_DASHBOARD_WORKSPACE:-personal-assistant}"

# Appended verbatim to every `claude` launch; word-split on spaces. Use it to
# load a private plugin channel, e.g.
#   Q_CLAUDE_EXTRA_ARGS='--dangerously-load-development-channels plugin:monitor@my-marketplace'
Q_CLAUDE_EXTRA_ARGS="${Q_CLAUDE_EXTRA_ARGS:-}"

# Opt in (1) to Codex's sandbox/approval bypass. Off by default: it hands the
# agent unrestricted execution on the host, which must never be a silent default.
Q_UNSAFE_CODEX="${Q_UNSAFE_CODEX:-0}"

# The claude model menu, shared by both launchers so the two stay in step.
# Order drives the menu; the map resolves a menu label to a --model value.
typeset -ga Q_AGENT_MODEL_ORDER
typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS

(( ${#Q_AGENT_MODEL_ORDER} )) || Q_AGENT_MODEL_ORDER=(
  'Opus'
  'OpusPlan (Sonnet)'
  'CCR'
  'Fable 5'
)

(( ${#Q_AGENT_MODELS} )) || Q_AGENT_MODELS=(
  'Opus'              'claude-opus-4-8'
  'OpusPlan (Sonnet)' 'opusplan'
  'CCR'               'CCR'          # not a model — dispatches to `ccr code`
  'Fable 5'           'claude-fable-5'
)

# Per-label extra flags, word-split on spaces.
(( ${#Q_AGENT_MODEL_ARGS} )) || Q_AGENT_MODEL_ARGS=(
  'OpusPlan (Sonnet)' '--effort medium'
)
