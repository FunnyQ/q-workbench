#!/usr/bin/env zsh
# Shared defaults for the workbench. Sourced by every script that reads a
# setting — never executed, so keep it side-effect free (no output, no `set`
# changes, no exits).
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
  # `|| true` is load-bearing: the callers run under `set -e`, and a missing
  # `herdr` on a minimal PATH would otherwise abort the sourcing script with 127
  # instead of falling through to the literal path below.
  Q_WORKBENCH_CONFIG_DIR="${Q_WORKBENCH_CONFIG_DIR:-$(herdr plugin config-dir q.workbench 2>/dev/null || true)}"
  : "${Q_WORKBENCH_CONFIG_DIR:=${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench}"
  Q_WORKBENCH_LOCAL_CONFIG="$Q_WORKBENCH_CONFIG_DIR/config.zsh"
fi
# Exported so re-sourcing in a child (the pickers respawn their source script on
# every fzf reload) skips the `herdr` shellout above.
export Q_WORKBENCH_CONFIG_DIR Q_WORKBENCH_LOCAL_CONFIG

[[ -r "$Q_WORKBENCH_LOCAL_CONFIG" ]] && source "$Q_WORKBENCH_LOCAL_CONFIG"

# Workspace the dashboard launcher opens its tab in. The launcher aborts with a
# notification when no workspace carries this label.
Q_DASHBOARD_WORKSPACE="${Q_DASHBOARD_WORKSPACE:-personal-assistant}"

# Appended verbatim to every harness launch; word-split on spaces, so no single
# argument may contain one. Both are empty by default — in particular the
# sandbox/approval bypass flags are opt-in, never a silent default:
#   Q_CLAUDE_EXTRA_ARGS='--dangerously-skip-permissions'
#   Q_CODEX_EXTRA_ARGS='--dangerously-bypass-approvals-and-sandbox'
Q_CLAUDE_EXTRA_ARGS="${Q_CLAUDE_EXTRA_ARGS:-}"
Q_CODEX_EXTRA_ARGS="${Q_CODEX_EXTRA_ARGS:-}"

# The claude model menu, shared by both launchers so the two stay in step.
# Order drives the menu; the map resolves a menu label to a --model value.
#
# The maps must be declared before the user file could have created them as
# plain arrays — `typeset -gA` on an existing plain array silently empties it.
# See config.example.zsh; a user file overriding these has to declare them too.
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

# Project registry. Q_PROJECTS_ROOT is the root of the `.git` sweep that feeds
# discovery alongside the Claude and Codex session histories.
Q_PROJECT_REGISTRY_FILE="${Q_PROJECT_REGISTRY_FILE:-$HOME/.local/state/herdr-projects/registry.json}"
Q_PROJECTS_ROOT="${Q_PROJECTS_ROOT:-$HOME/Projects}"

# SSH target registry. Q_SSH_CONFIG_FILE is what `sync` reconciles against and
# what the editor appends new hosts to; the history file seeds it once.
Q_SSH_REGISTRY_FILE="${Q_SSH_REGISTRY_FILE:-$HOME/.local/state/ssh-targets/registry.json}"
Q_SSH_CONFIG_FILE="${Q_SSH_CONFIG_FILE:-$HOME/.config/ssh/config}"
Q_SSH_HISTORY_FILE="${Q_SSH_HISTORY_FILE:-$HOME/.zsh_history}"
