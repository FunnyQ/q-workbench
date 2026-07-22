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

# Defaults: the unsafe flags stay off and the model map is populated.
Q_WORKBENCH_LOCAL_CONFIG="$tmp_dir/absent.zsh" zsh -c "
  source ${(q)config}
  [[ \$Q_UNSAFE_CODEX == 0 ]] || { print -u2 'Q_UNSAFE_CODEX defaults on'; exit 1 }
  [[ -z \$Q_CLAUDE_EXTRA_ARGS ]] || { print -u2 'Q_CLAUDE_EXTRA_ARGS defaults non-empty'; exit 1 }
  [[ -n \$Q_DASHBOARD_WORKSPACE ]] || { print -u2 'Q_DASHBOARD_WORKSPACE is empty'; exit 1 }
  (( \${#Q_AGENT_MODEL_ORDER} > 0 )) || { print -u2 'model order is empty'; exit 1 }
  for menu_label in \${Q_AGENT_MODEL_ORDER[@]}; do
    [[ -n \${Q_AGENT_MODELS[\$menu_label]:-} ]] || {
      print -u2 \"menu label has no model: \$menu_label\"; exit 1
    }
  done
"

# Environment beats the built-in default.
Q_WORKBENCH_LOCAL_CONFIG="$tmp_dir/absent.zsh" Q_UNSAFE_CODEX=1 zsh -c "
  source ${(q)config}
  [[ \$Q_UNSAFE_CODEX == 1 ]] || { print -u2 'env did not override the default'; exit 1 }
"

# The user config file beats the environment.
cat > "$local_config" <<'EOF'
Q_DASHBOARD_WORKSPACE='from-local-file'
Q_UNSAFE_CODEX=0
EOF
Q_WORKBENCH_LOCAL_CONFIG="$local_config" Q_UNSAFE_CODEX=1 \
  Q_DASHBOARD_WORKSPACE='from-env' zsh -c "
  source ${(q)config}
  [[ \$Q_DASHBOARD_WORKSPACE == from-local-file ]] || {
    print -u2 \"local file lost to env: \$Q_DASHBOARD_WORKSPACE\"; exit 1
  }
  [[ \$Q_UNSAFE_CODEX == 0 ]] || { print -u2 'local file could not turn the flag back off'; exit 1 }
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

print 'config: ok'
