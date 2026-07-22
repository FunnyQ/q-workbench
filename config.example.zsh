#!/usr/bin/env zsh
# Example user config for the q.workbench plugin. Copy it into place:
#
#   cp config.example.zsh "$(herdr plugin config-dir q.workbench)/config.zsh"
#
# Everything here is commented out — uncomment only what you want to change.
# This file is sourced by scripts/config.zsh *before* its defaults, so a plain
# assignment wins the `:-` fallbacks. It runs on every script start: keep it
# side-effect free (no output, no `set`, no `exit`).
#
# Use this file rather than ~/.zshrc — Herdr runs plugin actions detached, so an
# exported variable may never reach them.

# ── Dashboard ────────────────────────────────────────────────────────────────
# Workspace label the dashboard launcher opens its tab in. The launcher aborts
# with a notification when no workspace carries this label.
# Q_DASHBOARD_WORKSPACE='personal-assistant'

# ── Harness flags ────────────────────────────────────────────────────────────
# Appended verbatim to every launch of the matching harness; word-split on
# spaces, so no single argument may contain one.
#
# Both default to empty. The sandbox/approval bypass flags belong here and
# nowhere else: they hand the agent unrestricted execution on this host, so they
# are opt-in, per machine, and never a silent default.
# Q_CLAUDE_EXTRA_ARGS='--dangerously-load-development-channels plugin:monitor@my-marketplace'
# Q_CODEX_EXTRA_ARGS='--dangerously-bypass-approvals-and-sandbox'

# ── The claude model menu ────────────────────────────────────────────────────
# Three parallel structures, shared by both launch paths:
#   Q_AGENT_MODEL_ORDER  — menu order (also the pane/tab label)
#   Q_AGENT_MODELS       — label → --model value
#   Q_AGENT_MODEL_ARGS   — label → extra flags, word-split on spaces
#
# TRAP: declare the two maps with `typeset -gA` *before* assigning them, exactly
# as below. Without it they are created as plain arrays, and scripts/config.zsh's
# own `typeset -gA` silently empties them on conversion — the menu then falls
# back to the built-in models while your order stands, leaving labels that
# resolve to nothing.
#
# Setting these replaces the whole menu; it does not merge with the defaults.
# Any label you list in ORDER must have a matching key in MODELS.
#
# typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS
#
# Q_AGENT_MODEL_ORDER=(
#   'Opus'
#   'OpusPlan (Sonnet)'
#   'CCR'
#   'Fable 5'
# )
#
# Q_AGENT_MODELS=(
#   'Opus'              'claude-opus-4-8'
#   'OpusPlan (Sonnet)' 'opusplan'
#   'CCR'               'CCR'              # not a model — dispatches to `ccr code`
#   'Fable 5'           'claude-fable-5'
# )
#
# Q_AGENT_MODEL_ARGS=(
#   'OpusPlan (Sonnet)' '--effort medium'
# )

# ── Project registry ─────────────────────────────────────────────────────────
# Q_PROJECTS_ROOT is the root of the `.git` sweep that feeds discovery alongside
# the Claude and Codex session histories.
# Q_PROJECT_REGISTRY_FILE="$HOME/.local/state/herdr-projects/registry.json"
# Q_PROJECTS_ROOT="$HOME/Projects"

# ── SSH target registry ──────────────────────────────────────────────────────
# Q_SSH_CONFIG_FILE is what `sync` reconciles against and what the editor
# appends new hosts to. The history file seeds the registry once, on first sync.
# Q_SSH_REGISTRY_FILE="$HOME/.local/state/ssh-targets/registry.json"
# Q_SSH_CONFIG_FILE="$HOME/.config/ssh/config"
# Q_SSH_HISTORY_FILE="$HOME/.zsh_history"
