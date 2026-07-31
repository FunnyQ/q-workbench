# Q Workbench

A Herdr plugin (`q.workbench`) that turns a terminal multiplexer into an agent workbench: launch AI coding agents into a ready-made pane layout, jump between projects and SSH hosts with fuzzy pickers, and restart a stuck agent without losing its tab.

A Rust binary is committed to the repository, so installing requires no build. However, hacking on it requires a Rust toolchain.

## What it does

The plugin exposes six actions. Keys are yours to choose — see [Bind it](#bind-it).

| Action | What happens |
| --- | --- |
| `new-agent` | Pick harness → model → usage, then open a tab laid out as **agent \| yazi + terminal** |
| `new-worktree-agent` | Same, but first picks/creates a branch and starts every pane in a fresh `git worktree` |
| `project` | Fuzzy-find a registered project; focus its workspace or create one (falls back to `zoxide`) |
| `ssh` | Fuzzy-find an SSH host; connect in a dedicated tab that closes itself on disconnect |
| `restart-agent` | Confirm, then relaunch the agent **in place** — the yazi/terminal side panes survive |
| `dashboard` | Open a tab that starts Claude with the usage-dashboard prompt |

Harnesses offered: Claude Code (Opus / OpusPlan / CCR / Fable 5), Codex, opencode.

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

The bindings below are the set I use — copy them or pick your own:

| Key | Action |
| --- | --- |
| `alt+c` | `new-agent` |
| `alt+shift+c` | `new-worktree-agent` |
| `alt+p` | `project` |
| `alt+s` | `ssh` |
| `alt+r` | `restart-agent` |
| `prefix+d` | `dashboard` |

Pairing the two agent actions on `alt+c` / `alt+shift+c` is worth keeping whatever keys
you pick: the worktree-vs-normal choice *is* the keybinding, which is why neither action
prompts "use a worktree?".

### Requirements

macOS, Herdr ≥ 0.7.4, and on `PATH`: `gum`, `fzf`, `zoxide`, `yazi`.

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

Existing users can preview a migration from zsh to TOML, then write it after review:

```zsh
./bin/workbench config migrate
./bin/workbench config migrate --write
```

TOML arrays preserve argument boundaries. Put each flag and value in its own entry:

```toml
dashboard_workspace = "my-workspace"
claude_extra_args = ["--dangerously-load-development-channels", "plugin:monitor@my-marketplace"]
codex_extra_args = ["--dangerously-bypass-approvals-and-sandbox"]
```

| Setting | Default | Purpose |
| --- | --- | --- |
| `dashboard_workspace` | `personal-assistant` | Workspace the dashboard tab opens in |
| `claude_extra_args` | `[]` | Array appended to every `claude` launch |
| `codex_extra_args` | `[]` | Array appended to every `codex` launch |
| `order` / `models` / `model_args` | Opus, OpusPlan, CCR, Fable 5 | The Claude model menu; `order` and each `model_args` value are arrays |
| `project_registry_file` | `~/.local/state/herdr-projects/registry.json` | Project registry path |
| `projects_root` | `~/Projects` | Root of the `.git` discovery sweep |
| `ssh_registry_file` | `~/.local/state/ssh-targets/registry.json` | SSH registry path |
| `ssh_config_file` | `~/.config/ssh/config` | What `sync` reconciles against |
| `ssh_history_file` | `~/.zsh_history` | Seeds the SSH registry on first sync |

**On the bypass flags:** `--dangerously-bypass-approvals-and-sandbox` (Codex) and
`--dangerously-skip-permissions` (Claude) hand the agent unrestricted execution on your
host. Nothing adds them for you — put them in the `claude_extra_args` or
`codex_extra_args` TOML array deliberately, per machine.

## Development

```zsh
cargo test
cargo clippy -- -D warnings
zsh scripts/build.zsh
```

The binary must be rebuilt and committed as part of any release, since the version it reports comes from the `Cargo.toml` crate manifest.

See `CLAUDE.md` for architecture notes.
