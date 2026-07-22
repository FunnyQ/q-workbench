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

export ZSSH_REGISTRY_FILE="$tmpdir/targets.json"
export ZSSH_CONFIG_FILE="$tmpdir/config"
export ZSSH_HISTORY_FILE="$tmpdir/history"

"$script" sync
grep -Fx '  "version": 1,' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets | length == 4' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.source == "config"' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hostname == "configured.example"' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.user == "deploy"' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["old.example"].source == "manual"' "$ZSSH_REGISTRY_FILE" >/dev/null

"$script" use deploy@configured.example
jq -e '.targets["deploy@configured.example"] == null' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.last_used_at != null' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets | length == 3' "$ZSSH_REGISTRY_FILE" >/dev/null
# list emits NUL-separated multi-line records for fzf --read0 --gap; -z splits on NUL
"$script" list | grep -Fz $'configured\ndeploy@configured.example\n[config]\tconfigured' >/dev/null

"$script" remove configured
jq -e '.targets.configured.hidden == true' "$ZSSH_REGISTRY_FILE" >/dev/null
"$script" use new.example
jq -e '.targets["new.example"].source == "manual"' "$ZSSH_REGISTRY_FILE" >/dev/null
"$script" remove new.example
jq -e '.targets["new.example"] == null' "$ZSSH_REGISTRY_FILE" >/dev/null

"$script" use builder@new.example
ZSSH_EDIT_ALIAS=new-server ZSSH_EDIT_HOSTNAME=new.example \
    ZSSH_EDIT_USER=builder ZSSH_EDIT_PORT=2222 ZSSH_EDIT_CONFIRM=yes \
    "$editor_script" builder@new.example
grep -Fx 'Host new-server' "$ZSSH_CONFIG_FILE" >/dev/null
grep -Fx '  HostName new.example' "$ZSSH_CONFIG_FILE" >/dev/null
grep -Fx '  User builder' "$ZSSH_CONFIG_FILE" >/dev/null
grep -Fx '  Port 2222' "$ZSSH_CONFIG_FILE" >/dev/null
jq -e '.targets["builder@new.example"] == null' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["new-server"].source == "config"' "$ZSSH_REGISTRY_FILE" >/dev/null

cat > "$tmpdir/config" <<'EOF'
Host configured
  HostName updated.example
  User deploy
Host added
  HostName added.example
EOF
"$script" sync
jq -e '.targets.removable == null' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.added.source == "config"' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hidden == true' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets.configured.hostname == "updated.example"' "$ZSSH_REGISTRY_FILE" >/dev/null
jq -e '.targets["new-server"] == null' "$ZSSH_REGISTRY_FILE" >/dev/null

print -- "ssh-target-registry tests passed"
