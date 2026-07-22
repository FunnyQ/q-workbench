#!/usr/bin/env zsh
# config.zsh is sourced by every launcher, so a bad default or a broken
# precedence chain breaks all of them at once — and only at popup time.

set -eu

plugin_dir=${0:A:h:h}
config="$plugin_dir/scripts/config.zsh"

[[ -r "$config" ]] || {
  print -u2 "missing: $config"
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
local_config="$tmp_dir/user-config.zsh"

# Defaults: both extra-arg slots stay empty (the bypass flags are opt-in) and
# every menu label resolves to a model.
Q_WORKBENCH_LOCAL_CONFIG="$tmp_dir/absent.zsh" zsh -c "
  source ${(q)config}
  [[ -z \$Q_CLAUDE_EXTRA_ARGS ]] || { print -u2 'Q_CLAUDE_EXTRA_ARGS defaults non-empty'; exit 1 }
  [[ -z \$Q_CODEX_EXTRA_ARGS ]] || { print -u2 'Q_CODEX_EXTRA_ARGS defaults non-empty'; exit 1 }
  [[ -n \$Q_DASHBOARD_WORKSPACE ]] || { print -u2 'Q_DASHBOARD_WORKSPACE is empty'; exit 1 }
  (( \${#Q_AGENT_MODEL_ORDER} > 0 )) || { print -u2 'model order is empty'; exit 1 }
  for menu_label in \${Q_AGENT_MODEL_ORDER[@]}; do
    [[ -n \${Q_AGENT_MODELS[\$menu_label]:-} ]] || {
      print -u2 \"menu label has no model: \$menu_label\"; exit 1
    }
  done
  # Every path setting must land somewhere, or the script that reads it breaks
  # under `set -u` rather than falling back.
  for path_var in Q_PROJECT_REGISTRY_FILE Q_PROJECTS_ROOT \\
                  Q_SSH_REGISTRY_FILE Q_SSH_CONFIG_FILE Q_SSH_HISTORY_FILE; do
    [[ -n \${(P)path_var} ]] || { print -u2 \"\$path_var is empty\"; exit 1 }
  done
"

# Environment beats the built-in default.
Q_WORKBENCH_LOCAL_CONFIG="$tmp_dir/absent.zsh" Q_CODEX_EXTRA_ARGS='--from-env' zsh -c "
  source ${(q)config}
  [[ \$Q_CODEX_EXTRA_ARGS == --from-env ]] || { print -u2 'env did not override the default'; exit 1 }
"

# The user config file beats the environment.
cat > "$local_config" <<'EOF'
Q_DASHBOARD_WORKSPACE='from-local-file'
Q_CODEX_EXTRA_ARGS=''
Q_PROJECTS_ROOT='/from-local-file/projects'
EOF
Q_WORKBENCH_LOCAL_CONFIG="$local_config" Q_CODEX_EXTRA_ARGS='--from-env' \
  Q_DASHBOARD_WORKSPACE='from-env' Q_PROJECTS_ROOT='/from-env' zsh -c "
  source ${(q)config}
  [[ \$Q_DASHBOARD_WORKSPACE == from-local-file ]] || {
    print -u2 \"local file lost to env: \$Q_DASHBOARD_WORKSPACE\"; exit 1
  }
  [[ \$Q_PROJECTS_ROOT == /from-local-file/projects ]] || {
    print -u2 \"local file lost to env: \$Q_PROJECTS_ROOT\"; exit 1
  }
  [[ -z \$Q_CODEX_EXTRA_ARGS ]] || { print -u2 'local file could not clear the flag again'; exit 1 }
"

# Overriding the model menu from the user file only works if that file declares
# the maps `typeset -gA` first: converting an existing plain array empties it,
# the size check then sees 0 and silently restores the built-in menu — leaving
# labels from the user's order that resolve to no model at all.
cat > "$tmp_dir/models.zsh" <<'EOF'
typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS
Q_AGENT_MODEL_ORDER=( 'Mine' )
Q_AGENT_MODELS=( 'Mine' 'my-model' )
EOF
Q_WORKBENCH_LOCAL_CONFIG="$tmp_dir/models.zsh" zsh -c "
  source ${(q)config}
  [[ \$Q_AGENT_MODEL_ORDER == 'Mine' ]] || {
    print -u2 \"user model order was not honoured: \$Q_AGENT_MODEL_ORDER\"; exit 1
  }
  [[ \${Q_AGENT_MODELS[Mine]:-} == my-model ]] || {
    print -u2 'user model map was wiped by the typeset conversion'; exit 1
  }
  (( \${#Q_AGENT_MODELS} == 1 )) || {
    print -u2 'built-in models leaked into the user menu'; exit 1
  }
"

# With no CLI and no override, the path must still land on Herdr's documented
# per-plugin config dir — otherwise the user's config silently never loads.
resolved=$(PATH=/usr/bin:/bin HOME="$tmp_dir/home" XDG_CONFIG_HOME= zsh -c "
  source ${(q)config}
  print -r -- \$Q_WORKBENCH_LOCAL_CONFIG
")
[[ "$resolved" == "$tmp_dir/home/.config/herdr/plugins/config/q.workbench/config.zsh" ]] || {
  print -u2 "config path fallback resolves elsewhere: $resolved"
  exit 1
}

# When herdr IS on PATH, its answer wins over the literal fallback.
mkdir -p "$tmp_dir/bin"
cat > "$tmp_dir/bin/herdr" <<EOF
#!/bin/zsh
[[ "\$1 \$2" == 'plugin config-dir' ]] && print -r -- "$tmp_dir/from-cli"
EOF
chmod +x "$tmp_dir/bin/herdr"
resolved=$(PATH="$tmp_dir/bin:/usr/bin:/bin" zsh -c "
  source ${(q)config}
  print -r -- \$Q_WORKBENCH_LOCAL_CONFIG
")
[[ "$resolved" == "$tmp_dir/from-cli/config.zsh" ]] || {
  print -u2 "herdr plugin config-dir was ignored: $resolved"
  exit 1
}

# The registries and pickers run under `set -eu` on a minimal PATH. A failing
# `herdr` shellout inside config.zsh must fall through to the literal path, not
# abort the sourcing script with 127.
PATH=/usr/bin:/bin HOME="$tmp_dir/home" Q_WORKBENCH_CONFIG_DIR= zsh -c "
  set -eu
  source ${(q)config}
  [[ -n \$Q_PROJECTS_ROOT ]] || exit 1
" || {
  print -u2 'config.zsh aborts a `set -e` caller when herdr is off PATH'
  exit 1
}

print 'config: ok'
