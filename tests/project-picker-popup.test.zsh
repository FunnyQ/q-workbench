#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
picker="$plugin_dir/scripts/project-picker-popup.zsh"

[[ -x "$picker" ]] || {
  print -u2 "missing executable: $picker"
  exit 1
}

tmp_dir=$(mktemp -d)
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_bin="$tmp_dir/bin"
registry="$tmp_dir/registry.json"
log_file="$tmp_dir/herdr.log"
fzf_input="$tmp_dir/fzf-input"
fzf_args="$tmp_dir/fzf-args"
project="$tmp_dir/widget"
new_project="$tmp_dir/gadget"
zoxide_project="$tmp_dir/secret-lab"
mkdir -p "$mock_bin" "$project" "$new_project"
mkdir -p "$zoxide_project"
project=${project:A}
new_project=${new_project:A}
zoxide_project=${zoxide_project:A}

jq -n --arg project "$project" --arg new "$new_project" '{
  version: 1,
  generated_at: "2026-07-22T00:00:00Z",
  projects: {
    ($project): {name:"widget",sources:["filesystem"]},
    ($new): {name:"gadget",sources:["manual"]}
  }
}' > "$registry"

cat > "$mock_bin/fzf" <<'EOF'
#!/bin/zsh
[[ -z "${TEST_FZF_ARGS:-}" ]] || print -rl -- "$@" > "$TEST_FZF_ARGS"
if [[ -n "${TEST_FZF_INPUT:-}" ]]; then
  tee "$TEST_FZF_INPUT" >/dev/null
else
  cat >/dev/null
fi
print -r -- "${TEST_QUERY:-}"
print -r -- "${TEST_KEY:-}"
print -r -- "$TEST_PICK"
EOF


cat > "$mock_bin/zoxide" <<'EOF'
#!/bin/zsh
[[ "$1 $2 $3" == 'query -- secret' ]] || exit 1
print -r -- "$TEST_ZOXIDE_PATH"
EOF

cat > "$mock_bin/herdr" <<'EOF'
#!/bin/zsh
print -r -- "$*" >> "$TEST_LOG"
if [[ "$1 $2" == 'api snapshot' ]]; then
  jq -n --arg path "$TEST_OPEN_PATH" '{result:{snapshot:{panes:(if $path == "" then [] else [{workspace_id:"w9",cwd:$path,foreground_cwd:$path}] end)}}}'
elif [[ "$1 $2" == 'workspace create' ]]; then
  print '{"result":{"workspace":{"workspace_id":"w10"},"root_pane":{"pane_id":"w10:p1"},"tab":{"tab_id":"w10:t1"}}}'
fi
EOF

chmod +x "$mock_bin/fzf" "$mock_bin/herdr" "$mock_bin/zoxide"

PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" TEST_LOG="$log_file" \
  TEST_PICK="$project" TEST_OPEN_PATH="$project" TEST_FZF_INPUT="$fzf_input" \
  TEST_FZF_ARGS="$fzf_args" \
  HERDR_BIN_PATH="$mock_bin/herdr" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_PROJECTS_ROOT="$tmp_dir" "$picker"

rg -q '^workspace focus w9$' "$log_file"
rg -aFq -- '󰉋  widget' "$fzf_input"
rg -aFq -- "   $project" "$fzf_input"
rg -aFq -- '   filesystem' "$fzf_input"
rg -Fq -- 'ctrl-i:execute(' "$fzf_args"
rg -Fq -- 'ctrl-i: edit' "$fzf_args"
rg -Fq -- '--expect=alt-enter' "$fzf_args"
rg -Fq -- 'alt-enter: plain' "$fzf_args"
jq -e --arg project "$project" '.projects[$project].last_used_at | type == "number"' \
  "$registry" >/dev/null

: > "$log_file"
PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" TEST_LOG="$log_file" \
  TEST_PICK="$new_project" TEST_OPEN_PATH='' \
  HERDR_BIN_PATH="$mock_bin/herdr" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_PROJECTS_ROOT="$tmp_dir" "$picker"

rg -Fq -- "workspace create --cwd $new_project --label gadget --env Q_NO_BANNER=1 --no-focus" "$log_file"
rg -Fq -- 'tab rename w10:t1 󰧑  main' "$log_file"
rg -Fq -- 'pane rename w10:p1 󱚟  agent' "$log_file"
rg -Fq -- 'pane run w10:p1 ' "$log_file"
rg -q '^workspace focus w10$' "$log_file"

: > "$log_file"
PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" TEST_LOG="$log_file" \
  TEST_PICK="$new_project" TEST_KEY='alt-enter' TEST_OPEN_PATH='' \
  HERDR_BIN_PATH="$mock_bin/herdr" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_PROJECTS_ROOT="$tmp_dir" "$picker"

rg -Fq -- "workspace create --cwd $new_project --label gadget --env Q_NO_BANNER=1 --no-focus" "$log_file"
rg -q '^workspace focus w10$' "$log_file"
if rg -q '^(tab rename|pane rename|pane run) ' "$log_file"; then
  print -u2 'plain workspace unexpectedly built the agent layout'
  exit 1
fi

: > "$log_file"
PATH="$mock_bin:/opt/homebrew/bin:/usr/bin:/bin" TEST_LOG="$log_file" \
  TEST_PICK='' TEST_QUERY='secret' TEST_ZOXIDE_PATH="$zoxide_project" TEST_OPEN_PATH='' \
  HERDR_BIN_PATH="$mock_bin/herdr" \
  Q_PROJECT_REGISTRY_FILE="$registry" Q_PROJECTS_ROOT="$tmp_dir" "$picker"

rg -Fq -- "workspace create --cwd $zoxide_project --label secret-lab --env Q_NO_BANNER=1 --no-focus" "$log_file"
jq -e --arg project "$zoxide_project" '
  .projects[$project].sources == ["manual"] and
  (.projects[$project].last_used_at | type == "number")
' "$registry" >/dev/null

print 'project-picker-popup: ok'
