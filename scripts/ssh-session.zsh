#!/usr/bin/env zsh
# Own one SSH connection and close its dedicated Herdr tab on exit.

export PATH="$PATH:/opt/homebrew/bin"

target="$1"
tab_id="$2"
registry="${Q_SSH_REGISTRY:-${0:A:h}/ssh-target-registry.zsh}"

close_tab() {
  herdr tab close "$tab_id" >/dev/null 2>&1
}
trap close_tab EXIT HUP INT TERM

ssh "$target"
ssh_status=$?

if (( ssh_status == 0 )); then
  "$registry" use "$target"
  print -r -- ": $(date +%s):0;ssh $target" >> "$HOME/.zsh_history"
fi

exit "$ssh_status"
