Q_DASHBOARD_WORKSPACE='personal-assistant'
Q_CLAUDE_EXTRA_ARGS='--permission-mode plan'
Q_CODEX_EXTRA_ARGS='--search --profile work'
Q_PROJECT_REGISTRY_FILE="$HOME/.local/state/custom-projects/registry.json"
Q_PROJECTS_ROOT="$HOME/Projects"
Q_SSH_REGISTRY_FILE="$HOME/.local/state/custom-ssh/registry.json"
Q_SSH_CONFIG_FILE="$HOME/.ssh/config"

typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS

Q_AGENT_MODEL_ORDER=(
  'Opus'
  'OpusPlan (Sonnet)'
  'CCR'
  'Fable 5'
)

Q_AGENT_MODELS=(
  'Opus' 'claude-opus-4-8'
  'OpusPlan (Sonnet)' 'opusplan'
  'CCR' 'CCR'
  'Fable 5' 'claude-fable-5'
)

Q_AGENT_MODEL_ARGS=(
  'OpusPlan (Sonnet)' '--effort medium'
  'Fable 5' '--fast'
)
