#!/usr/bin/env zsh
# The other ssh tests always inject Q_SSH_REGISTRY_SCRIPT / Q_SSH_EDITOR, so the
# ${0:A:h} fallbacks never run there — a typo in one would keep them green and
# only surface when the popup is opened. Eval the real assignment lines out of
# the sources so a typo fails here.

set -eu

plugin_dir=${0:A:h:h}
scripts_dir="$plugin_dir/scripts"

check() {
  local src="$1" var="$2" line resolved
  line=$(grep -m1 -E "^${var}=\"\\\$\{Q_SSH_" "$src") || {
    print -u2 "$src: no ${var}= assignment with a Q_SSH_ default"
    exit 1
  }

  # $0 is what ${0:A:h} resolves against inside the script under test
  0="$src" Q_SSH_REGISTRY_SCRIPT= Q_SSH_EDITOR= eval "$line"
  resolved=${(P)var}

  [[ -x "$resolved" ]] || {
    print -u2 "$src: ${var} default is not an executable file: $resolved"
    exit 1
  }
}

check "$scripts_dir/ssh-picker-popup.zsh" registry
check "$scripts_dir/ssh-picker-popup.zsh" editor
check "$scripts_dir/ssh-session.zsh" registry

print "ssh-script-defaults: ok"
