#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
session_script="$plugin_dir/scripts/ssh-session.zsh"
tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_bin="$tmp_dir/bin"
log_file="$tmp_dir/session.log"
mkdir -p "$mock_bin"

cat > "$mock_bin/ssh" <<'EOF'
#!/bin/zsh
print -r -- "ssh $*" >> "$TEST_LOG"
exit 0
EOF

cat > "$mock_bin/herdr" <<'EOF'
#!/bin/zsh
print -r -- "herdr $*" >> "$TEST_LOG"
EOF

cat > "$tmp_dir/registry" <<'EOF'
#!/bin/zsh
print -r -- "registry $*" >> "$TEST_LOG"
EOF

chmod +x "$mock_bin/ssh" "$mock_bin/herdr" "$tmp_dir/registry"

PATH="$mock_bin:/usr/bin:/bin" HOME="$tmp_dir" TEST_LOG="$log_file" \
  Q_SSH_REGISTRY="$tmp_dir/registry" "$session_script" example-host 1:3

actual=$(<"$log_file")
[[ "$actual" == $'ssh example-host\nregistry use example-host\nherdr tab close 1:3' ]]
[[ -s "$tmp_dir/.zsh_history" ]]
[[ "$(<"$tmp_dir/.zsh_history")" == *';ssh example-host' ]]

print 'ssh-session: ok'
