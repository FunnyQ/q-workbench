# Q Workbench

A Herdr plugin (`q.workbench`) that turns a terminal multiplexer into an agent workbench: launch AI coding agents into a ready-made pane layout, jump between projects and SSH hosts with fuzzy pickers, and restart a stuck agent without losing its tab.

A Rust binary is committed to the repository, so installing requires no build. However, hacking on it requires a Rust toolchain.

> **macOS only for now.** The committed `bin/workbench` is a single Mach-O **arm64** artifact, and `herdr-plugin.toml` declares `platforms = ["macos"]`. It will not run on Linux, and it will not run on an Intel Mac. Building from source on another platform is untested. Shipping a second artifact is out of scope for this release.

## What it does

The plugin exposes nine actions. Keys are yours to choose — see [Bind it](#bind-it).

| Action | What happens |
| --- | --- |
| `new-agent` | Pick harness → model → usage, then open a tab laid out as **agent \| yazi + terminal** |
| `new-worktree-agent` | Same, but first picks/creates a branch and starts every pane in a fresh `git worktree` |
| `new-assistant` | Open a tab from the `personal-assistant` layout — every choice pinned, so no menus |
| `new-tab` | Pick a tab layout — a blank tab is always the last row — then the menus that layout leaves open, and open a tab from it |
| `project` | Fuzzy-find a project; focus its workspace or create one. Typing widens the list past the registry, into `zoxide` and a live sweep of `projects_root` |
| `ssh` | Fuzzy-find an SSH host; connect in a dedicated tab that closes itself on disconnect |
| `restart-agent` | Confirm, then relaunch the agent **in place** — the yazi/terminal side panes survive |
| `dashboard` | Open a tab that starts Claude with the usage-dashboard prompt |
| `even-out-panes` | Even out the split ratios in the focused pane's row or column, leaving any orthogonal split (e.g. a Files/term stack in one slot) untouched |

The defaults offer Claude Code (Opus / OpusPlan / CCR / Fable 5), Codex, and opencode; agents and their options are configurable.

## Install

```zsh
herdr plugin install FunnyQ/q-workbench     # optionally --ref <tag-or-branch>
herdr plugin list                           # confirm q.workbench is enabled
```

To hack on it instead, clone anywhere and link the checkout:

```zsh
git clone https://github.com/FunnyQ/q-workbench.git
herdr plugin link ./q-workbench
```

`link` registers the repo in place. Edits to Rust or shell scripts require running `zsh scripts/build.zsh` to rebuild the embedded binary before changes take effect. This is the single most likely source of confusing stale-binary bugs.

### Bind it

The plugin ships no keybindings — every action is reachable from Herdr's action list,
and you bind whichever ones you want in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "alt+c"                          # your choice
type = "plugin_action"
command = "q.workbench.new-agent"      # <plugin id>.<action id>
description = "new agent"
```

The bindings below are the set I use, copy them or pick your own:

| Key | Action |
| --- | --- |
| `alt+c` | `new-agent` |
| `alt+shift+c` | `new-worktree-agent` |
| `alt+t` | `new-tab` |
| `alt+p` | `project` |
| `alt+s` | `ssh` |
| `alt+r` | `restart-agent` |
| `prefix+d` | `dashboard` |
| `prefix+e` | `even-out-panes` |

Pairing the two agent actions on `alt+c` / `alt+shift+c` is worth keeping whatever keys
you pick: the worktree-vs-normal choice *is* the keybinding, which is why neither action
prompts "use a worktree?". `new-tab` asks which layout instead and still does not ask
about worktrees.

### Requirements

macOS on Apple silicon, Herdr ≥ 0.7.4, and on `PATH`: `gum`, `fzf`, `zoxide`, `yazi`.

```zsh
brew install gum fzf zoxide yazi
```

## Registries

Both pickers read a JSON registry you can regenerate at any time.

**Projects** — `~/.local/state/herdr-projects/registry.json`

```zsh
./bin/workbench project scan          # first run: discover and review
./bin/workbench project rescan        # re-review, marking [new] / [missing]
./bin/workbench project update        # refresh sources, no prompts
./bin/workbench project edit <path>   # rename, add aliases, hide
```

Discovery pulls from Claude Code sessions, Codex rollouts, and a `.git` sweep of `~/Projects`. Entries sort by most-recently-used.

Type two characters and the picker also sweeps `projects_root` live, listing its finds below the registry rows. A directory counts when it holds `.git` or a `project_markers` file. Finding one does not register it, picking it does, so the registry holds only the projects you actually open.

**SSH targets** — `~/.local/state/ssh-targets/registry.json`

```zsh
./bin/workbench ssh sync       # reconcile against ~/.config/ssh/config
./bin/workbench ssh list
```

Hosts come from your SSH config (seeded once from shell history). In the picker, `ctrl-i` edits a host (adding new ones to your SSH config), `ctrl-x` removes it.

## Configuration

Machine-specific values use TOML outside this repo, so they survive a reinstall and
cannot be committed by accident. Create this file when you need an override:

```text
~/.config/herdr/plugins/config/q.workbench/config.toml
```

`Q_WORKBENCH_LOCAL_CONFIG` overrides the resolved config path. Point it at a TOML file;
the binary rejects a path with a `.zsh` extension and explains how to correct it.

The config contains `[[tab_layouts]]` entries with nested panes and `[[agents]]`
entries with nested options. Omitting a layout choice makes the launcher ask for it.
For each layout, `label` sets the layout menu row text and falls back to `name` when
omitted. `icon` is drawn before the label, separated by two spaces, and falls back to
no icon when omitted. Two layouts may not have the same rendered menu row, and a rendered
row may not start or end with whitespace. Both rules apply to `[[agents]]` too.

A layout may declare no agent pane, one, or several, at any position. Each agent pane
that pins neither `agent` nor `option` runs its own harness and model menu, in the order
the panes are written; the usage menu then runs once for the tab. A layout with no agent
pane asks for a plain tab name instead — submitting nothing keeps its `label`.

`blank-tab` is a reserved layout name. The `new-tab` menu offers a blank tab as its last
row whether or not the config declares one, so declare that section only to change what
blank opens.

This minimal config keeps the built-in layout and defines one agent option:

```toml
dashboard_workspace = "my-workspace"
default_tab_layout = "agentic-coding"

[[agents]]
name = "claude code"
command = ["claude"]

  [[agents.options]]
  name = "Opus"
  args = ["--model", "claude-opus-4-8"]
```

See [`config.example.toml`](config.example.toml) for the full schema and built-in
defaults.

| Setting | Default | Purpose |
| --- | --- | --- |
| `dashboard_workspace` | `personal-assistant` | Workspace the dashboard tab opens in |
| `default_tab_layout` | `agentic-coding` | Layout used when a launch does not pass `--layout` — the `project`, `new-agent`, and `new-worktree-agent` actions, and an in-pane `agent launch`. The `new-tab` action also hoists it to the top of its menu, above the blank row pinned to the bottom |
| `project_registry_file` | `~/.local/state/herdr-projects/registry.json` | Project registry path |
| `projects_root` | `~/Projects` | Root of the `.git` discovery sweep, and of the picker's live sweep |
| `project_markers` | `package.json`, `Gemfile`, `Cargo.toml`, `CLAUDE.md` | File names that make a directory a project in the picker's sweep, alongside `.git`. Set to `[]` to leave `.git` as the only marker |
| `ssh_registry_file` | `~/.local/state/ssh-targets/registry.json` | SSH registry path |
| `ssh_config_file` | `~/.config/ssh/config` | What `sync` reconciles against |
| `ssh_history_file` | `~/.zsh_history` | Seeds the SSH registry on first sync |

**On the bypass flags:** `--dangerously-bypass-approvals-and-sandbox` (Codex) and
`--dangerously-skip-permissions` (Claude) hand the agent unrestricted execution on your
host. Nothing adds them for you — put them in the agent's `extra_args` array
deliberately, per machine.

## Development

```zsh
cargo test
cargo clippy -- -D warnings
zsh scripts/build.zsh
```

The binary must be rebuilt and committed as part of any release, since the version it reports comes from the `Cargo.toml` crate manifest.

See `CLAUDE.md` for architecture notes.
