# Q Workbench

A Herdr plugin (`q.workbench`) that turns a terminal multiplexer into an agent workbench: launch AI coding agents into a ready-made pane layout, jump between projects and SSH hosts with fuzzy pickers, and restart a stuck agent without losing its tab.

Pure zsh. No build step, no dependencies to install beyond the CLI tools below.

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

`link` registers the repo in place — edits to `scripts/` take effect on the next invocation, no reinstall.

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

macOS, Herdr ≥ 0.7.4, and on `PATH`: `jq`, `gum`, `fzf`, `zoxide`, `rg`, `yazi`, `trash`.

```zsh
brew install jq gum fzf zoxide ripgrep yazi trash
```

## Registries

Both pickers read a JSON registry you can regenerate at any time.

**Projects** — `~/.local/state/herdr-projects/registry.json`

```zsh
scripts/project-registry.zsh scan          # first run: discover and review
scripts/project-registry.zsh rescan        # re-review, marking [new] / [missing]
scripts/project-registry.zsh update        # refresh sources, no prompts
scripts/project-registry.zsh edit <path>   # rename, add aliases, hide
```

Discovery pulls from Claude Code sessions, Codex rollouts, and a `.git` sweep of `~/Projects`. Entries sort by most-recently-used.

**SSH targets** — `~/.local/state/ssh-targets/registry.json`

```zsh
scripts/ssh-target-registry.zsh sync       # reconcile against ~/.config/ssh/config
scripts/ssh-target-registry.zsh list
```

Hosts come from your SSH config (seeded once from shell history). In the picker, `ctrl-i` edits a host (adding new ones to your SSH config), `ctrl-x` removes it.

## Configuration

Machine-specific values go in Herdr's per-plugin config dir — outside this repo, so
they survive a reinstall and can't be committed by accident:

```zsh
cp config.example.zsh "$(herdr plugin config-dir q.workbench)/config.zsh"
$EDITOR "$(herdr plugin config-dir q.workbench)/config.zsh"
```

`config.example.zsh` in this repo documents every setting, fully commented out.

It is sourced by `scripts/config.zsh` — which every script that reads a setting sources
in turn — ahead of every default, so plain assignments win:

```zsh
Q_DASHBOARD_WORKSPACE='my-workspace'
Q_CLAUDE_EXTRA_ARGS='--dangerously-load-development-channels plugin:monitor@my-marketplace'
Q_CODEX_EXTRA_ARGS='--dangerously-bypass-approvals-and-sandbox'
```

Use the file rather than `~/.zshrc`: Herdr runs plugin actions detached, so an exported
variable may not reach them.

| Variable | Default | Purpose |
| --- | --- | --- |
| `Q_DASHBOARD_WORKSPACE` | `personal-assistant` | Workspace the dashboard tab opens in |
| `Q_CLAUDE_EXTRA_ARGS` | *(empty)* | Appended to every `claude` launch |
| `Q_CODEX_EXTRA_ARGS` | *(empty)* | Appended to every `codex` launch |
| `Q_AGENT_MODEL_ORDER` / `Q_AGENT_MODELS` / `Q_AGENT_MODEL_ARGS` | Opus, OpusPlan, CCR, Fable 5 | The claude model menu — declare the maps `typeset -gA` first, see the example file |
| `Q_PROJECT_REGISTRY_FILE` | `~/.local/state/herdr-projects/registry.json` | |
| `Q_PROJECTS_ROOT` | `~/Projects` | Root of the `.git` discovery sweep |
| `Q_SSH_REGISTRY_FILE` | `~/.local/state/ssh-targets/registry.json` | |
| `Q_SSH_CONFIG_FILE` | `~/.config/ssh/config` | What `sync` reconciles against |
| `Q_SSH_HISTORY_FILE` | `~/.zsh_history` | Seeds the SSH registry on first sync |

**On the bypass flags:** `--dangerously-bypass-approvals-and-sandbox` (Codex) and
`--dangerously-skip-permissions` (Claude) hand the agent unrestricted execution on your
host. Nothing adds them for you — put them in the `*_EXTRA_ARGS` slot deliberately, per
machine.

## Development

```zsh
zsh tests/project-registry.test.zsh                       # one test
for t in tests/*.test.zsh; do zsh "$t" || break; done      # all of them
```

Tests are standalone zsh scripts — no framework. Each builds a `mktemp -d` sandbox, shims `herdr`/`fzf`/`gum`/`ssh` into a mock `PATH`, and asserts with `jq -e`. A non-zero exit is a failure.

See `CLAUDE.md` for architecture notes.
