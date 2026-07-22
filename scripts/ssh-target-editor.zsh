#!/usr/bin/env zsh
set -u

target="${1:-}"
config="${ZSSH_CONFIG_FILE:-$HOME/.config/ssh/config}"
registry="${0:A:h}/ssh-target-registry.zsh"

[[ -n "$target" ]] || { print -u2 -- "No SSH target selected."; exit 1; }
target_data=$("$registry" get "$target") || exit 1
source_type=$(jq -r '.source' <<< "$target_data")

if [[ "$source_type" == "config" ]]; then
    exec "${VISUAL:-${EDITOR:-nvim}}" "$config"
fi

default_user=""
default_hostname="$target"
if [[ "$target" == *@* ]]; then
    default_user="${target%%@*}"
    default_hostname="${target#*@}"
fi

if [[ -n "${ZSSH_EDIT_ALIAS:-}" ]]; then
    alias_name="$ZSSH_EDIT_ALIAS"
    hostname="${ZSSH_EDIT_HOSTNAME:-$default_hostname}"
    user="${ZSSH_EDIT_USER:-$default_user}"
    port="${ZSSH_EDIT_PORT:-22}"
    confirmed="${ZSSH_EDIT_CONFIRM:-no}"
else
    command -v gum >/dev/null || { print -u2 -- "gum is required for SSH target editing."; exit 1; }
    # fzf already owns the alternate screen. Clearing it gives the editor the
    # full pane while allowing fzf to redraw correctly when this command exits.
    clear
    alias_name=$(gum input --header="Create SSH config" --prompt="Alias › " --placeholder="server-name") || exit 0
    hostname=$(gum input --prompt="HostName › " --value="$default_hostname") || exit 0
    user=$(gum input --prompt="User › " --value="$default_user") || exit 0
    port=$(gum input --prompt="Port › " --value="22") || exit 0

    print
    print -- "Host $alias_name"
    print -- "  HostName $hostname"
    [[ -n "$user" ]] && print -- "  User $user"
    print -- "  Port $port"
    print
    gum confirm "Add this host to SSH config?" || exit 0
    confirmed=yes
fi

[[ "$confirmed" == "yes" ]] || exit 0
print -r -- "$alias_name" | grep -qxE '[A-Za-z0-9_.-]+' || { print -u2 -- "Invalid SSH alias: $alias_name"; exit 1; }
[[ -n "$hostname" && "$hostname" != *[[:space:]]* ]] || { print -u2 -- "Invalid HostName: $hostname"; exit 1; }
[[ -z "$user" ]] || print -r -- "$user" | grep -qxE '[A-Za-z0-9_.-]+' || { print -u2 -- "Invalid SSH user: $user"; exit 1; }
print -r -- "$port" | grep -qxE '[0-9]+' && (( port >= 1 && port <= 65535 )) || { print -u2 -- "Invalid SSH port: $port"; exit 1; }

if [[ -f "$config" ]] && awk -v wanted="$alias_name" '
    tolower($1) == "host" { for (i=2; i<=NF; i++) if ($i == wanted) found=1 }
    END { exit !found }
' "$config"; then
    print -u2 -- "SSH alias already exists: $alias_name"
    exit 1
fi

mkdir -p "${config:h}"
tmp=$(mktemp "${config}.XXXXXX") || exit 1
trap '[[ -f "$tmp" ]] && trash "$tmp"' EXIT
[[ -f "$config" ]] && cp -p "$config" "$tmp"
{
    [[ ! -s "$tmp" ]] || print
    print -- "Host $alias_name"
    print -- "  HostName $hostname"
    [[ -n "$user" ]] && print -- "  User $user"
    print -- "  Port $port"
} >> "$tmp"
mv "$tmp" "$config"
trap - EXIT

"$registry" sync
"$registry" use "$target"
print -- "Added SSH config: $alias_name"
