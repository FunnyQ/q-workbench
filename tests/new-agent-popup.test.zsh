#!/usr/bin/env zsh

set -eu

plugin_dir=${0:A:h:h}
popup_script="$plugin_dir/scripts/new-agent-popup.zsh"

[[ -x "$popup_script" ]] || {
  print -u2 "missing executable: $popup_script"
  exit 1
}

tmp_dir=${$(mktemp -d):A}   # :A resolves the /var -> /private/var symlink so it matches $PWD
trap 'trash "$tmp_dir" 2>/dev/null || true' EXIT
mock_bin="$tmp_dir/bin"
log_file="$tmp_dir/herdr.log"
mkdir -p "$mock_bin"

cat > "$mock_bin/gum" <<'EOF'
#!/bin/zsh
case "$1" in
  style) shift; print -r -- "$*" ;;
  choose)
    [[ -n "$TEST_CANCEL" ]] && exit 1
    count_file="${TEST_TMP_DIR}/gum-count"
    count=$(($(cat "$count_file" 2>/dev/null || print 0) + 1))
    print "$count" > "$count_file"
    case "$count" in
      1) print "codex" ;;
      2) print "review" ;;
    esac
    ;;
esac
EOF

cat > "$mock_bin/herdr" <<'EOF'
#!/bin/zsh
print -r -- "$*" >> "$TEST_LOG"
case "$1 $2" in
  "tab create") print '{"result":{"root_pane":{"pane_id":"1-1"},"tab":{"tab_id":"1:2"}}}' ;;
  "pane split")
    if [[ "$*" == *"1-1"* ]]; then
      print '{"result":{"pane":{"pane_id":"1-2"}}}'
    else
      print '{"result":{"pane":{"pane_id":"1-3"}}}'
    fi
    ;;
esac
EOF

chmod +x "$mock_bin/gum" "$mock_bin/herdr"

# run outside any git repo so project_dir falls back to $PWD instead of the
# ambient toplevel (the plugin lives inside a repo during development)
(cd "$tmp_dir" && PATH="$mock_bin:/usr/bin:/bin" HERDR_WORKSPACE_ID= \
  TEST_TMP_DIR="$tmp_dir" TEST_LOG="$log_file" Q_WORKBENCH_LOCAL_CONFIG=/dev/null \
  "$popup_script")

actual=$(<"$log_file")
expected="tab create --label review --cwd $tmp_dir --env Q_NO_BANNER=1 --no-focus
pane rename 1-1 review
tab rename 1:2 review
pane split 1-1 --direction right --ratio 0.38 --cwd $tmp_dir --env Q_NO_BANNER=1 --no-focus
pane rename 1-2 󰥨  Files
pane run 1-2 yazi .
pane split 1-2 --direction down --ratio 0.9 --cwd $tmp_dir --no-focus
pane rename 1-3   term
pane run 1-1 codex
tab focus 1:2"

[[ "$actual" == "$expected" ]] || {
  print -u2 "unexpected command sequence"
  diff -u <(print -r -- "$expected") <(print -r -- "$actual")
  exit 1
}

# The sandbox bypass must be opt-in, never a silent default.
> "$log_file"
trash "$tmp_dir/gum-count" 2>/dev/null || true
(cd "$tmp_dir" && PATH="$mock_bin:/usr/bin:/bin" HERDR_WORKSPACE_ID= \
  TEST_TMP_DIR="$tmp_dir" TEST_LOG="$log_file" Q_WORKBENCH_LOCAL_CONFIG=/dev/null \
  Q_UNSAFE_CODEX=1 "$popup_script")
grep -qxF 'pane run 1-1 codex --dangerously-bypass-approvals-and-sandbox' "$log_file" || {
  print -u2 'Q_UNSAFE_CODEX=1 did not add the bypass flag'
  exit 1
}

> "$log_file"
trash "$tmp_dir/gum-count" 2>/dev/null || true
PATH="$mock_bin:/usr/bin:/bin" HERDR_WORKSPACE_ID= \
  TEST_TMP_DIR="$tmp_dir" TEST_LOG="$log_file" Q_WORKBENCH_LOCAL_CONFIG=/dev/null \
  TEST_CANCEL=1 "$popup_script"
[[ ! -s "$log_file" ]] || {
  print -u2 'cancel created a Herdr resource'
  exit 1
}

print "new-agent-popup: ok"
