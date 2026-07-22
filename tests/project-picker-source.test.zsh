#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
source_script="$plugin_dir/scripts/project-picker-source.zsh"

[[ -x "$source_script" ]] || {
  print -u2 "missing executable: $source_script"
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_bin="$tmp_dir/bin"
registry="$tmp_dir/registry.json"
zoxide_project="$tmp_dir/odd-nuxt-ts-starter"
output="$tmp_dir/output"
mkdir -p "$mock_bin" "$zoxide_project"
zoxide_project=${zoxide_project:A}

jq -n --arg visible "$tmp_dir/visible" --arg hidden "$tmp_dir/hidden" '{
  version: 1,
  projects: {
    ($visible): {name:"Visible App", aliases:["find-me", "secondary"], sources:["manual"]},
    ($hidden): {name:"Hidden App", hidden:true, sources:["manual"]}
  }
}' > "$registry"
cat > "$mock_bin/zoxide" <<'EOF'
#!/bin/zsh
[[ "$1 $2 $3" == 'query -- starter' ]] || exit 1
print -r -- "$TEST_ZOXIDE_PATH"
EOF
chmod +x "$mock_bin/zoxide"

PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" \
  TEST_ZOXIDE_PATH="$zoxide_project" Q_PROJECT_REGISTRY_FILE="$registry" \
  "$source_script" starter > "$output"

rg -aFq -- '󰉋  odd-nuxt-ts-starter' "$output"
rg -aFq -- "   $zoxide_project" "$output"
rg -aFq -- $'   zoxide\t' "$output"
rg -aFq -- '󰉋  Visible App | find-me | secondary' "$output"
if rg -aFq -- '   find-me · secondary' "$output"; then
  print -u2 'aliases unexpectedly rendered on a separate line'
  exit 1
fi
if rg -aFq -- 'Hidden App' "$output"; then
  print -u2 'hidden project unexpectedly rendered'
  exit 1
fi

print 'project-picker-source: ok'
