#!/usr/bin/env zsh
set -u

source "${0:A:h}/config.zsh"

registry="$Q_SSH_REGISTRY_FILE"
config="$Q_SSH_CONFIG_FILE"
history_file="$Q_SSH_HISTORY_FILE"

write_registry() {
    local content="$1" tmp
    mkdir -p "${registry:h}"
    tmp=$(mktemp "${registry}.XXXXXX") || return 1
    print -r -- "$content" | jq '.' > "$tmp" || {
        trash "$tmp"
        return 1
    }
    mv "$tmp" "$registry"
}

config_alias_groups() {
    [[ -f "$config" ]] || return 0
    awk 'tolower($1) == "host" {
        group = ""
        for (i = 2; i <= NF; i++) if ($i !~ /[*!?]/) group = group (group ? " " : "") $i
        if (group) print group
    }' "$config" | sort -u
}

config_records() {
    local group effective hostname user primary
    local -a aliases
    while IFS= read -r group; do
        [[ -n "$group" ]] || continue
        aliases=(${=group})
        primary="${aliases[1]}"
        effective=$(ssh -G -F "$config" -- "$primary" 2>/dev/null) || continue
        hostname=$(print -r -- "$effective" | awk '$1 == "hostname" { print $2; exit }')
        user=$(print -r -- "$effective" | awk '$1 == "user" { print $2; exit }')
        jq -cn --arg target "$primary" --arg hostname "$hostname" --arg user "$user" \
            '{target:$target,hostname:$hostname,user:$user,aliases:$ARGS.positional}' \
            --args "${aliases[@]}"
    done < <(config_alias_groups)
}

history_targets() {
    [[ -f "$history_file" ]] || return 0
    grep -E '^: [0-9]+:[0-9]+;(TERM=[^ ]+ )?ssh ' "$history_file" 2>/dev/null | \
        sed 's/^: \([0-9]*\):[0-9]*;.*ssh \(-[^ ]* \)*/\1 /' | \
        sort -rn | awk 'seen[$2]++ == 0 { print $2 }' | \
        grep -xE '[][a-zA-Z0-9_.@-]+' || true
}

sync_registry() {
    local data='{"version":1,"targets":{}}' target config_json
    if [[ -s "$registry" ]] && jq -e '.version == 1 and (.targets | type == "object")' "$registry" >/dev/null 2>&1; then
        data=$(<"$registry")
    else
        while IFS= read -r target; do
            [[ -n "$target" ]] || continue
            data=$(jq -c --arg target "$target" '.targets[$target] = {source:"manual",last_used_at:null,hidden:false}' <<< "$data")
        done < <(history_targets)
    fi

    config_json=$(config_records | jq -sc '.')
    data=$(jq -c --argjson configured "$config_json" '
        ($configured | map(.target)) as $configured_targets |
        .targets |= with_entries(select(.value.source != "config" or (.key as $k | $configured_targets | index($k)))) |
        reduce $configured[] as $record (.;
            if .targets[$record.target] then
                .targets[$record.target].source = "config" |
                .targets[$record.target].hostname = $record.hostname |
                .targets[$record.target].user = $record.user |
                .targets[$record.target].aliases = $record.aliases
            else
                .targets[$record.target] = ($record + {source:"config",last_used_at:null,hidden:false} | del(.target))
            end)
    ' <<< "$data")
    write_registry "$data"
}

# Items are multi-line, so records are NUL-delimited for fzf --read0.
# jq emits \f as the record separator and tr rewrites it to NUL.
list_targets() {
    sync_registry || return 1
    jq -j '.targets | to_entries | map(select(.value.hidden != true)) |
        sort_by(if .value.last_used_at then [0, -(.value.last_used_at)] else [1, .key] end)[] |
        if .value.source == "config" then
            "\((.value.aliases // [.key]) | join("  "))\n\(.value.user)@\(.value.hostname)\n[config]\t\(.key)\f"
        else
            "\(.key)\n[manual]\t\(.key)\f"
        end' "$registry" | tr '\014' '\000'
}

# A config Host may declare several aliases; the registry is keyed by the first one.
resolve_alias() {
    jq -r --arg target "$1" '
        if .targets[$target] then $target
        else ([.targets | to_entries[] |
            select(.value.aliases // [] | index($target)) | .key][0] // $target) end' "$registry"
}

use_target() {
    local target="$1" data target_user="" target_host now=$(date +%s)
    sync_registry || return 1
    target=$(resolve_alias "$target")
    target_host="$target"
    if [[ "$target" == *@* ]]; then
        target_user="${target%%@*}"
        target_host="${target#*@}"
    fi
    data=$(jq -c --arg target "$target" --arg user "$target_user" --arg host "$target_host" --argjson now "$now" '
        [.targets | to_entries[] |
            select(.value.source == "config" and .value.hostname == $host and
                ($user == "" or .value.user == $user)) | .key] as $matches |
        if .targets[$target].source == "config" then
            .targets[$target].last_used_at=$now | .targets[$target].hidden=false
        elif $matches | length == 1 then
            del(.targets[$target]) |
            .targets[$matches[0]].last_used_at=$now |
            .targets[$matches[0]].hidden=false
        elif .targets[$target] then
            .targets[$target].last_used_at=$now | .targets[$target].hidden=false
        else
            .targets[$target]={source:"manual",last_used_at:$now,hidden:false}
        end' "$registry")
    write_registry "$data"
}

remove_target() {
    local target="$1" data
    sync_registry || return 1
    data=$(jq -c --arg target "$target" '
        if .targets[$target].source == "config" then .targets[$target].hidden=true
        else del(.targets[$target]) end' "$registry")
    write_registry "$data"
}

get_target() {
    local target="$1"
    sync_registry || return 1
    jq -e --arg target "$target" '.targets[$target]' "$registry"
}

case "${1:-list}" in
    sync) sync_registry ;;
    list) list_targets ;;
    use) [[ -n "${2:-}" ]] && use_target "$2" ;;
    remove) [[ -n "${2:-}" ]] && remove_target "$2" ;;
    get) [[ -n "${2:-}" ]] && get_target "$2" ;;
    *) print -u2 -- "Usage: ${0:t} {sync|list|get TARGET|use TARGET|remove TARGET}"; exit 2 ;;
esac
