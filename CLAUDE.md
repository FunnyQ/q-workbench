# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A **Herdr plugin** (`q.workbench`) — pure zsh, no build step, no package manager. It ships terminal-multiplexer actions for Q's workflow: launching AI agents in structured tab layouts, picking projects/SSH targets via fzf, and restarting agents in place.

`herdr-plugin.toml` is the entry point: `[[actions]]` (what appears in Herdr's action list) and `[[panes]]` (popup panes an action can open). Every `command` is `["zsh", "scripts/<name>.zsh"]` — adding a feature means adding a script *and* registering it in the manifest.

## Commands

```zsh
zsh tests/project-registry.test.zsh     # run a single test
for t in tests/*.test.zsh; do zsh "$t" || break; done   # run all tests
```

There is no test framework. Each `tests/*.test.zsh` is a standalone script that `set -eu`, builds a `mktemp -d` sandbox, shims external binaries (`herdr`, `fzf`, `gum`, `zoxide`, `ssh`) into a `$mock_bin` prepended to `PATH`, runs the script under test, asserts with `jq -e`, and prints `<name>: ok`. Non-zero exit = failure. Follow that shape exactly for new tests.

## Architecture

### Two agent-launch paths — know which one you're touching

Both build the same 3-pane layout (agent | yazi "Files" / term) but from opposite sides:

- **`scripts/new-agent-popup.zsh`** — runs *inside a popup*. Collects all choices first (worktree → harness → model → usage), **then** creates the tab and panes via `herdr tab create` / `pane split` / `pane run`. Has a `cleanup_tab()` that closes the half-built tab and notifies on any failure.
- **`scripts/agent-launcher.zsh`** — runs *inside the agent pane itself* and ends in `exec <harness>`. Menus render at full pane width first; the yazi/term split is deliberately deferred to the very end so no resize happens mid-menu and the chosen worktree drives `--cwd` for all three panes. `scripts/build-agent-tab.zsh` is the thin wrapper that injects it into a pane (used by `ccc` in `zsh/functions/herdr.zsh`, outside this repo).

`agent-launcher.zsh` takes positional args `<pane_id> [tab_id] [fixed_usage] [wt_mode] [layout_mode]`; empty-but-quoted slots matter (`'' '' ''`) since args don't shift.

**Restart-in-place** (`restart-agent.zsh`) depends on `exec` replacing the *launcher* subprocess, not the pane's shell — so killing the agent's foreground process group drops the pane back to its prompt instead of destroying it. It then re-injects the launcher with `layout_mode=no-layout`. It also `stty sane`s and clears Kitty keyboard-protocol state because Codex leaves the TTY dirty on SIGTERM.

### Configuration

`scripts/config.zsh` is **sourced** by all three launchers — it must stay side-effect free (no output, no `set`, no `exit`). Precedence is user config → environment → built-in defaults, achieved by sourcing the user file *first* so its plain assignments survive the `:-` fallbacks.

The user file lives at `$(herdr plugin config-dir q.workbench)/config.zsh`, resolved lazily (the CLI is only shelled out to when `Q_WORKBENCH_LOCAL_CONFIG` is unset) with a literal `~/.config/herdr/plugins/config/…` fallback. **Tests that invoke a launcher must pass `Q_WORKBENCH_LOCAL_CONFIG=/dev/null`** — otherwise the developer's real config leaks in and assertions about defaults pass or fail by machine.

It owns the claude model menu (`Q_AGENT_MODEL_ORDER` / `Q_AGENT_MODELS` / `Q_AGENT_MODEL_ARGS`) so the two launchers can't drift apart, plus `Q_CLAUDE_EXTRA_ARGS` and `Q_UNSAFE_CODEX`.

Harness bypass flags (`--dangerously-bypass-approvals-and-sandbox`) are **opt-in**. Do not reintroduce them as unconditional defaults; `tests/new-agent-popup.test.zsh` asserts both states.

### Registries

Two JSON state files, both `version: 1`, both written atomically (`mktemp` → `jq '.'` → `mv`) via a local `write_registry`:

- **Projects** — `~/.local/state/herdr-projects/registry.json` (override: `$Q_PROJECT_REGISTRY_FILE`). `project-registry.zsh {scan|rescan|update|use PATH|edit PATH}`. Discovery merges three sources: Claude sessions (`~/.claude/projects`), Codex rollouts (`~/.codex/sessions`), and a `.git` sweep of `$Q_PROJECTS_ROOT` (default `~/Projects`). `canonical_project()` resolves to the git toplevel and drops temp dirs — keep that filter intact.
- **SSH targets** — `~/.local/state/ssh-targets/registry.json` (override: `$ZSSH_REGISTRY_FILE`). `ssh-target-registry.zsh {sync|list|get|use|remove}`. `sync` reconciles against `~/.config/ssh/config` (`$ZSSH_CONFIG_FILE`) using `ssh -G`; config-sourced entries are *hidden* on remove, manual ones deleted. Seeded once from `~/.zsh_history`.

`_source` fields accumulate; `use` stamps `last_used_at`, which drives picker sort order.

### fzf pickers

`project-picker-popup.zsh` and `ssh-picker-popup.zsh` both feed fzf **NUL-delimited multi-line records** (`--read0`) with a tab-separated payload (`--delimiter=$'\t' --with-nth=1 --accept-nth=2`). `list_targets` produces those by emitting `\f` from jq and `tr`-ing it to NUL. Keybindings are wired as `execute(...)+reload(...)` back into the same scripts, so editors must `clear` before drawing (fzf owns the alternate screen).

The project picker also falls back to `zoxide query` when the typed query matches no registered project.

## Conventions

- `#!/usr/bin/env zsh`, `set -eu` where the script isn't a menu flow, `export PATH="/opt/homebrew/bin:$PATH"` at the top (plugin actions run detached with a minimal PATH).
- Hard dependencies, assumed present: `herdr`, `jq`, `gum`, `fzf`, `zoxide`, `rg`, `trash`.
- `trash`, never `rm` — including in test cleanup traps.
- Every `herdr` call is parsed with `jq -r '.result...// empty'` and guarded; failures are surfaced through `herdr notification show`, not stdout.
- Menu labels carry Nerd Font glyphs and are also the pane/tab label — stripping the leading pad (`${x#"${x%%[![:space:]]*}"}`) is intentional, keeping the glyph is too.
- Comments in this repo explain *why* (ordering traps, TTY quirks, git-worktree constraints). Match that density; don't strip them.
