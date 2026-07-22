#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
picker="$plugin_dir/scripts/ssh-picker-popup.zsh"

[[ -x "$picker" ]] || {
  print -u2 "missing executable: $picker"
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_bin="$tmp_dir/bin"
log_file="$tmp_dir/herdr.log"
mkdir -p "$mock_bin"

cat > "$mock_bin/fzf" <<'EOF'
#!/bin/zsh
cat >/dev/null
print
print 'example-host'
EOF

cat > "$mock_bin/herdr" <<'EOF'
#!/bin/zsh
print -r -- "$*" >> "$TEST_LOG"
if [[ "$1 $2" == 'tab create' ]]; then
  print '{"result":{"root_pane":{"pane_id":"1-4"},"tab":{"tab_id":"1:3"}}}'
fi
EOF

cat > "$tmp_dir/registry" <<'EOF'
#!/bin/zsh
print -rn -- $'Example\texample-host\0'
EOF

chmod +x "$mock_bin/fzf" "$mock_bin/herdr" "$tmp_dir/registry"

PATH="$mock_bin:/usr/bin:/bin" TEST_LOG="$log_file" \
  Q_SSH_REGISTRY_SCRIPT="$tmp_dir/registry" Q_SSH_EDITOR=/bin/true \
  HERDR_WORKSPACE_ID=w18 "$picker"

actual=$(<"$log_file")
[[ "$actual" == *'tab create --workspace w18 --label 󰢩  example-host'* ]]
[[ "$actual" == *'pane run 1-4 '*'/ssh-session.zsh example-host 1:3'* ]]
[[ "$actual" == *$'\ntab focus 1:3' ]]

print 'ssh-picker-popup: ok'
