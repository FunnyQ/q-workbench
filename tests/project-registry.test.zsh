#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
registry_script="$plugin_dir/scripts/project-registry.zsh"

[[ -x "$registry_script" ]] || {
  print -u2 "missing executable: $registry_script"
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_home="$tmp_dir/home"
mock_bin="$tmp_dir/bin"
registry="$tmp_dir/state/herdr-projects/registry.json"
project="$mock_home/Projects/acme/widget"
new_project="$mock_home/Projects/acme/gadget"
missing_project="$mock_home/Projects/acme/missing"
temporary_cwd="$tmp_dir/transient-session"
gum_input="$tmp_dir/gum-input"

mkdir -p "$mock_bin" "$project" "$temporary_cwd" \
  "$mock_home/.claude/projects/empty" \
  "$mock_home/.claude/projects/widget" \
  "$mock_home/.codex/sessions/2026/07/22"
git -C "$project" init -q
project=${project:A}

cat > "$mock_home/.claude/projects/widget/sessions-index.json" <<EOF
{"version":1,"entries":[{"projectPath":"$project/app"}]}
EOF
cat > "$mock_home/.claude/projects/widget/session.jsonl" <<EOF
{"type":"user","cwd":"$project/app"}
EOF
mkdir -p "$project/app"

cat > "$mock_home/.codex/sessions/2026/07/22/rollout-root.jsonl" <<'EOF'
{"type":"session_meta","payload":{"cwd":"/"}}
{"type":"event_msg","payload":{"type":"user_message"}}
EOF

cat > "$mock_home/.codex/sessions/2026/07/22/rollout-test.jsonl" <<EOF
{"type":"session_meta","payload":{"cwd":"$project"}}
{"type":"event_msg","payload":{"type":"user_message"}}
EOF

cat > "$mock_home/.codex/sessions/2026/07/22/rollout-temporary.jsonl" <<EOF
{"type":"session_meta","payload":{"cwd":"$temporary_cwd"}}
EOF

cat > "$mock_bin/gum" <<'EOF'
#!/bin/zsh
[[ "${Q_GUM_CANCEL:-}" != 1 ]] || exit 1
if [[ "$1" == input ]]; then
  case "$*" in
    *'Display name'*) print -r -- "${Q_EDIT_NAME:-Widget App}" ;;
    *'Aliases'*) print -r -- "${Q_EDIT_ALIASES:-widgets, widget-app}" ;;
  esac
  exit 0
elif [[ "$1" == choose && "$*" != *--no-limit* ]]; then
  print -r -- "${Q_EDIT_VISIBILITY:-hidden}"
  exit 0
fi
while IFS= read -r line; do
  [[ -z "${Q_GUM_INPUT:-}" ]] || print -r -- "$line" >> "$Q_GUM_INPUT"
  print -r -- "$line"
done
EOF
chmod +x "$mock_bin/gum"

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" scan

if ! jq -e --arg project "$project" '
  .version == 1 and
  (.projects | keys) == [$project] and
  .projects[$project].name == "widget" and
  .projects[$project].sources == ["claude", "codex", "filesystem"]
' "$registry" >/dev/null; then
  jq '.' "$registry" >&2
  exit 1
fi

if HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" scan 2>/dev/null; then
  print -u2 'scan unexpectedly overwrote an existing registry'
  exit 1
fi

tmp_registry=$(mktemp "${registry}.XXXXXX")
jq --arg project "$project" --arg missing "$missing_project" '
  .projects[$project].aliases = ["widgets"] |
  .projects[$project].last_used_at = 123 |
  .projects[$missing] = {
    name: "missing",
    aliases: ["old"],
    sources: ["filesystem"]
  }
' "$registry" > "$tmp_registry"
mv "$tmp_registry" "$registry"

mkdir -p "$new_project"
git -C "$new_project" init -q
new_project=${new_project:A}

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_GUM_INPUT="$gum_input" \
  "$registry_script" rescan

jq -e --arg project "$project" --arg new "$new_project" --arg missing "$missing_project" '
  .projects[$project].aliases == ["widgets"] and
  .projects[$project].last_used_at == 123 and
  .projects[$new].sources == ["filesystem"] and
  .projects[$missing].aliases == ["old"]
' "$registry" >/dev/null
rg -q '^\[new\] gadget\t' "$gum_input"
rg -q '^\[missing\] missing\t' "$gum_input"

before_cancel=$(shasum -a 256 "$registry" | awk '{print $1}')
if HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_GUM_CANCEL=1 \
  "$registry_script" rescan 2>/dev/null; then
  print -u2 'cancelled rescan unexpectedly succeeded'
  exit 1
fi
after_cancel=$(shasum -a 256 "$registry" | awk '{print $1}')
[[ "$before_cancel" == "$after_cancel" ]]

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" use "$project"
jq -e --arg project "$project" '
  .projects[$project].last_used_at | type == "number"
' "$registry" >/dev/null
used_at=$(jq -r --arg project "$project" '.projects[$project].last_used_at' "$registry")

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" edit "$project"
jq -e --arg project "$project" '
  .projects[$project].name == "Widget App" and
  .projects[$project].aliases == ["widgets", "widget-app"] and
  .projects[$project].hidden == true and
  .projects[$project].sources == ["claude", "codex", "filesystem"]
' "$registry" >/dev/null

before_cancel=$(shasum -a 256 "$registry" | awk '{print $1}')
if HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_GUM_CANCEL=1 \
  "$registry_script" edit "$project" 2>/dev/null; then
  print -u2 'cancelled edit unexpectedly succeeded'
  exit 1
fi
after_cancel=$(shasum -a 256 "$registry" | awk '{print $1}')
[[ "$before_cancel" == "$after_cancel" ]]

if HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" edit "$new_project/unknown" \
  2>/dev/null; then
  print -u2 'edit unexpectedly accepted an unknown project'
  exit 1
fi

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" rescan
jq -e --arg project "$project" '
  .projects[$project].name == "Widget App" and
  .projects[$project].aliases == ["widgets", "widget-app"] and
  .projects[$project].hidden == true
' "$registry" >/dev/null

trash "$mock_home/.claude/projects/widget/sessions-index.json"
tmp_registry=$(mktemp "${registry}.XXXXXX")
jq --arg project "$project" '
  .projects[$project].sources = ["filesystem"]
' "$registry" > "$tmp_registry"
mv "$tmp_registry" "$registry"

HOME="$mock_home" PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  Q_PROJECT_REGISTRY_FILE="$registry" "$registry_script" update
jq -e --arg project "$project" --argjson used_at "$used_at" '
  .projects[$project].sources == ["claude", "codex", "filesystem"] and
  .projects[$project].name == "Widget App" and
  .projects[$project].aliases == ["widgets", "widget-app"] and
  .projects[$project].hidden == true and
  .projects[$project].last_used_at == $used_at
' "$registry" >/dev/null

print 'project-registry: ok'
