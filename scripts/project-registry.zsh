#!/usr/bin/env zsh
# Discover local projects, review them interactively, then write the registry.

set -eu

registry="${Q_PROJECT_REGISTRY_FILE:-$HOME/.local/state/herdr-projects/registry.json}"
projects_root="${Q_PROJECTS_ROOT:-$HOME/Projects}"

canonical_project() {
  local project_path="$1" root physical_projects_root
  [[ -d "$project_path" ]] || return 0
  [[ "$project_path" != / ]] || return 0

  root=$(git -C "$project_path" rev-parse --show-toplevel 2>/dev/null) || root="$project_path"
  [[ "$root" != / ]] || return 0
  root=$(cd -P -- "$root" && pwd)
  physical_projects_root=${projects_root:A}
  if [[ "$root" != "$physical_projects_root"/* ]] && \
    { [[ "$root" == /tmp || "$root" == /tmp/* || "$root" == /private/tmp || "$root" == /private/tmp/* ]] || \
      [[ "$root" == /var/folders/*/*/T || "$root" == /var/folders/*/*/T/* ]] || \
      [[ "$root" == /private/var/folders/*/*/T || "$root" == /private/var/folders/*/*/T/* ]]; }; then
    return 0
  fi
  print -r -- "$root"
}

claude_records() {
  setopt localoptions noerrexit
  local project_path project
  [[ -d "$HOME/.claude/projects" ]] || return 0

  while IFS= read -r project_path; do
    [[ -n "$project_path" ]] || continue
    project=$(canonical_project "$project_path")
    [[ -n "$project" ]] && print -r -- "$project"$'\t''claude'
  done < <({
    find "$HOME/.claude/projects" -name sessions-index.json -type f \
      -exec jq -r '.entries[]?.projectPath // empty' {} + 2>/dev/null
    { rg -m1 --no-filename -g '*.jsonl' -o '"cwd":"([^"\\]|\\.)*"' \
      "$HOME/.claude/projects" 2>/dev/null || true; } | \
      jq -Rr '("{" + . + "}" | fromjson? | .cwd // empty)'
  } | sort -u)
  return 0
}

codex_records() {
  setopt localoptions noerrexit
  local project_path project
  [[ -d "$HOME/.codex/sessions" ]] || return 0

  while IFS= read -r project_path; do
    [[ -n "$project_path" ]] || continue
    project=$(canonical_project "$project_path")
    [[ -n "$project" ]] && print -r -- "$project"$'\t''codex'
  done < <(find "$HOME/.codex/sessions" -name 'rollout-*.jsonl' -type f \
    -exec awk 'FNR == 1 { print }' {} + 2>/dev/null | jq -Rr '
      fromjson? |
      if .type == "session_meta" then (.payload.cwd // .cwd // empty)
      else (.cwd // empty)
      end
    ' | sort -u)
  return 0
}

filesystem_records() {
  setopt localoptions noerrexit
  local marker project
  [[ -d "$projects_root" ]] || return 0

  while IFS= read -r -d $'\0' marker; do
    project=$(canonical_project "${marker:h}")
    [[ -n "$project" ]] && print -r -- "$project"$'\t''filesystem'
  done < <(find "$projects_root" \
    -type d \( -name node_modules -o -name vendor -o -name tmp -o -name log \
      -o -name coverage -o -name dist -o -name build -o -name .nuxt -o -name .next \) \
      -prune -o -name .git -prune -print0 2>/dev/null)
  return 0
}

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

use_project() {
  local project_path project data content now

  [[ -f "$registry" ]] || {
    print -u2 "project-registry: registry does not exist: $registry"
    return 1
  }
  project_path="${1:-}"
  [[ -n "$project_path" ]] || {
    print -u2 'project-registry: project path is required'
    return 1
  }
  project=$(canonical_project "$project_path")
  [[ -n "$project" ]] || {
    print -u2 "project-registry: invalid project path: $project_path"
    return 1
  }
  data=$(jq -c 'select(.version == 1 and (.projects | type == "object"))' "$registry")
  [[ -n "$data" ]] || {
    print -u2 "project-registry: invalid registry: $registry"
    return 1
  }

  now=$(date +%s)
  content=$(jq -c --arg project "$project" --argjson now "$now" '
    .projects[$project] = ((.projects[$project] // {
      name: ($project | split("/") | last),
      sources: ["manual"]
    }) + {last_used_at: $now})
  ' <<< "$data")
  write_registry "$content"
}

edit_project() {
  local project data current_name current_aliases current_visibility
  local name aliases visibility content

  command -v gum >/dev/null || {
    print -u2 'project-registry: gum is required'
    return 1
  }
  command -v jq >/dev/null || {
    print -u2 'project-registry: jq is required'
    return 1
  }
  [[ -f "$registry" ]] || {
    print -u2 "project-registry: registry does not exist: $registry"
    return 1
  }
  project="${1:-}"
  [[ "$project" == /* ]] || {
    print -u2 'project-registry: absolute project path is required'
    return 1
  }
  data=$(jq -c 'select(.version == 1 and (.projects | type == "object"))' "$registry")
  [[ -n "$data" ]] || {
    print -u2 "project-registry: invalid registry: $registry"
    return 1
  }
  jq -e --arg project "$project" '.projects[$project] != null' <<< "$data" \
    >/dev/null || {
      print -u2 "project-registry: project is not registered: $project"
      return 1
    }

  current_name=$(jq -r --arg project "$project" \
    '.projects[$project].name // ($project | split("/") | last)' <<< "$data")
  current_aliases=$(jq -r --arg project "$project" \
    '(.projects[$project].aliases // []) | join(", ")' <<< "$data")
  current_visibility=$(jq -r --arg project "$project" \
    'if .projects[$project].hidden == true then "hidden" else "visible" end' <<< "$data")

  name=$(gum input --header='Display name' --value="$current_name") || {
    print -u2 'project-registry: edit cancelled; registry not written'
    return 1
  }
  aliases=$(gum input --header='Aliases (comma-separated)' --value="$current_aliases") || {
    print -u2 'project-registry: edit cancelled; registry not written'
    return 1
  }
  visibility=$(gum choose visible hidden --header='Picker visibility' \
    --selected="$current_visibility") || {
      print -u2 'project-registry: edit cancelled; registry not written'
      return 1
    }
  [[ "$visibility" == visible || "$visibility" == hidden ]] || {
    print -u2 'project-registry: invalid visibility; registry not written'
    return 1
  }

  content=$(jq -c --arg project "$project" --arg name "$name" \
    --arg aliases "$aliases" --arg visibility "$visibility" '
    def trim: sub("^[[:space:]]+"; "") | sub("[[:space:]]+$"; "");
    ($name | trim) as $clean_name |
    ($aliases | split(",") | map(trim) | map(select(length > 0)) |
      reduce .[] as $alias ([]; if index($alias) then . else . + [$alias] end)) as $clean_aliases |
    .projects[$project].name = (
      if $clean_name == "" then ($project | split("/") | last) else $clean_name end
    ) |
    .projects[$project].aliases = $clean_aliases |
    .projects[$project].hidden = ($visibility == "hidden")
  ' <<< "$data")
  write_registry "$content"
  print -r -- "project-registry: edited ${project}"
}

update_registry() {
  local records data content now

  command -v jq >/dev/null || {
    print -u2 'project-registry: jq is required'
    return 1
  }
  [[ -f "$registry" ]] || {
    print -u2 "project-registry: registry does not exist: $registry"
    return 1
  }
  data=$(jq -c 'select(.version == 1 and (.projects | type == "object"))' "$registry")
  [[ -n "$data" ]] || {
    print -u2 "project-registry: invalid registry: $registry"
    return 1
  }

  records=$({ claude_records; codex_records; filesystem_records; } | sort -u)
  now=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
  content=$(jq -c --arg generated_at "$now" --arg records "$records" '
    ($records | split("\n") | map(select(length > 0) | split("\t")) |
      reduce .[] as $row ({};
        .[$row[0]] = ((.[$row[0]] // []) + [$row[1]] | unique)
      )) as $discovered |
    .generated_at = $generated_at |
    .projects |= with_entries(
      .value.sources = (
        (($discovered[.key] // []) +
          [(.value.sources // [])[] | select(. == "manual")]) | unique
      )
    )
  ' <<< "$data")

  write_registry "$content"
  print -r -- "project-registry: updated ${registry}"
}

scan_registry() {
  local mode="$1" records candidates selected content now existing

  command -v gum >/dev/null || {
    print -u2 'project-registry: gum is required'
    return 1
  }
  command -v jq >/dev/null || {
    print -u2 'project-registry: jq is required'
    return 1
  }
  if [[ "$mode" == scan ]]; then
    [[ ! -e "$registry" ]] || {
      print -u2 "project-registry: registry already exists: $registry"
      return 1
    }
    existing='{"version":1,"projects":{}}'
  else
    [[ -f "$registry" ]] || {
      print -u2 "project-registry: registry does not exist: $registry"
      return 1
    }
    existing=$(jq -c '
      select(.version == 1 and (.projects | type == "object"))
    ' "$registry")
    [[ -n "$existing" ]] || {
      print -u2 "project-registry: invalid registry: $registry"
      return 1
    }
  fi

  records=$({ claude_records; codex_records; filesystem_records; } | sort -u)
  [[ -n "$records" ]] || {
    print -u2 'project-registry: no projects found'
    return 1
  }

  candidates=$(jq -Rrn --arg mode "$mode" --arg records "$records" \
    --argjson existing "$existing" '
    ($records | split("\n") | map(split("\t")) |
      reduce .[] as $row ({}; .[$row[0]] = true)) as $discovered |
    (($discovered + ($existing.projects | map_values(true))) | keys[]) as $path |
    ($path | split("/") | last) as $name |
    if $mode == "scan" then "\($name)\t\($path)"
    elif ($existing.projects[$path] == null) then "[new] \($name)\t\($path)"
    elif ($discovered[$path] != true) then "[missing] \($name)\t\($path)"
    else "\($name)\t\($path)"
    end
  ')

  selected=$(print -r -- "$candidates" | gum choose --no-limit --selected='*' \
    --ordered --height=24 --header='Review projects (space: toggle · enter: save)' \
    --no-strip-ansi) || {
      print -u2 'project-registry: cancelled; registry not written'
      return 1
    }
  selected=$(print -r -- "$selected" | awk -F '\t' 'NF >= 2 { print $NF }')
  [[ -n "$selected" ]] || {
    print -u2 'project-registry: nothing selected; registry not written'
    return 1
  }

  now=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
  content=$(jq -Rn --arg generated_at "$now" --arg records "$records" \
    --argjson existing "$existing" '
    ($records | split("\n") | map(split("\t")) |
      reduce .[] as $row ({};
        .[$row[0]] = ((.[$row[0]] // []) + [$row[1]] | unique)
      )) as $sources |
    [inputs | select(length > 0)] as $selected |
    {
      version: 1,
      generated_at: $generated_at,
      projects: reduce $selected[] as $path ({};
        .[$path] = (($existing.projects[$path] // {}) + {
          name: ($existing.projects[$path].name // ($path | split("/") | last)),
          sources: ($sources[$path] // $existing.projects[$path].sources // [])
        })
      )
    }
  ' <<< "$selected")

  write_registry "$content"
  print -r -- "project-registry: wrote ${registry}"
}

case "${1:-}" in
  scan) scan_registry scan ;;
  rescan) scan_registry rescan ;;
  update) update_registry ;;
  use) use_project "${2:-}" ;;
  edit) edit_project "${2:-}" ;;
  *) print -u2 -- "Usage: ${0:t} {scan|rescan|update|use PATH|edit PATH}"; exit 2 ;;
esac
