#!/usr/bin/env zsh
set -euo pipefail

script="${0:A:h:h}/scripts/ssh-target-registry.zsh"
editor_script="${0:A:h:h}/scripts/ssh-target-editor.zsh"
tmpdir=$(mktemp -d)
trap '_exit_code=$?; trash "$tmpdir"; exit $_exit_code' EXIT

cat > "$tmpdir/config" <<'EOF'
Host configured
  HostName configured.example
  User deploy
Host removable
  HostName removable.example
EOF
cat > "$tmpdir/history" <<'EOF'
: 100:0;ssh old.example
: 200:0;ssh configured
: 300:0;ssh deploy@configured.example
EOF

export Q_SSH_REGISTRY_FILE="$tmpdir/targets.json"
export Q_SSH_CONFIG_FILE="$tmpdir/config"
export Q_SSH_HISTORY_FILE="$tmpdir/history"

"$script" sync
grep -Fx '  "version": 1,' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets | length == 4' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.source == "config"' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hostname == "configured.example"' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.user == "deploy"' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["old.example"].source == "manual"' "$Q_SSH_REGISTRY_FILE" >/dev/null

"$script" use deploy@configured.example
jq -e '.targets["deploy@configured.example"] == null' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.last_used_at != null' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets | length == 3' "$Q_SSH_REGISTRY_FILE" >/dev/null
# list emits NUL-separated multi-line records for fzf --read0 --gap; -z splits on NUL
"$script" list | grep -Fz $'configured\ndeploy@configured.example\n[config]\tconfigured' >/dev/null

"$script" remove configured
jq -e '.targets.configured.hidden == true' "$Q_SSH_REGISTRY_FILE" >/dev/null
"$script" use new.example
jq -e '.targets["new.example"].source == "manual"' "$Q_SSH_REGISTRY_FILE" >/dev/null
"$script" remove new.example
jq -e '.targets["new.example"] == null' "$Q_SSH_REGISTRY_FILE" >/dev/null

"$script" use builder@new.example
Q_SSH_EDIT_ALIAS=new-server Q_SSH_EDIT_HOSTNAME=new.example \
    Q_SSH_EDIT_USER=builder Q_SSH_EDIT_PORT=2222 Q_SSH_EDIT_CONFIRM=yes \
    "$editor_script" builder@new.example
grep -Fx 'Host new-server' "$Q_SSH_CONFIG_FILE" >/dev/null
grep -Fx '  HostName new.example' "$Q_SSH_CONFIG_FILE" >/dev/null
grep -Fx '  User builder' "$Q_SSH_CONFIG_FILE" >/dev/null
grep -Fx '  Port 2222' "$Q_SSH_CONFIG_FILE" >/dev/null
jq -e '.targets["builder@new.example"] == null' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["new-server"].source == "config"' "$Q_SSH_REGISTRY_FILE" >/dev/null

cat > "$tmpdir/config" <<'EOF'
Host configured
  HostName updated.example
  User deploy
Host added
  HostName added.example
EOF
"$script" sync
jq -e '.targets.removable == null' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.added.source == "config"' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hidden == true' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hostname == "updated.example"' "$Q_SSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["new-server"] == null' "$Q_SSH_REGISTRY_FILE" >/dev/null

print -- "ssh-target-registry tests passed"
