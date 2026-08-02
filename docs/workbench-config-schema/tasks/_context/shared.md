# Shared context

> All tasks reference this. Decisions here override anything inferred from the codebase.

## Project at a glance

`q.workbench` is a Herdr plugin implemented as a single Rust binary. It launches AI agents into structured tab layouts, picks projects and SSH targets, and restarts agents in place. `herdr-plugin.toml` registers the actions and popup panes; most entries run the committed `bin/workbench` artifact.

This plan replaces five flat config fields with two declarative TOML sections, so tab layouts and harnesses become config-driven instead of hardcoded.

## Tech stack

- **Language**: Rust 2021.
- **Config**: `toml 0.8` + `serde` (both already in `Cargo.toml`). No new dependencies.
- **Errors**: `anyhow` — `bail!` / `Context` / `with_context`.
- **Transport**: a Unix socket at `$HERDR_SOCKET_PATH`, newline-delimited JSON, one request per connection.
- **Tests**: `#[cfg(test)] mod tests` inside each source file. `FakeClient` (in `src/herdr/`) replays queued Herdr responses and records every call.

## Code style

- Match the surrounding code. It is dense, comment-light, and comments explain **why**, never what.
- Comments are reserved for ordering, protocol, terminal, and quoting reasons that are not obvious from the code. Do not add a comment that restates the line below it.
- `clippy` runs with `-D warnings`. No `#[allow]` unless the surrounding code already uses one for the same reason.
- Errors that a user sees must name the concrete cause. `bail!("model order label has no model entry: {label}")` is the house style — the offending value is always in the message.
- Prefer a longer function over an indirection used once.
- Authoritative sources (for verification only): `Cargo.toml`, `CLAUDE.md`.

## File / directory layout

- `src/config.rs` — TOML loading, defaults, validation.
- `src/flows/agent.rs` — the launch flow: menus, `AgentChoice`, pane building.
- `src/flows/restart.rs` — in-place restart via a detached worker.
- `src/flows/picker.rs` — project and SSH fzf pickers.
- `src/flows/dashboard.rs`, `src/flows/layout.rs` — **not touched by this plan**.
- `src/state.rs` — the `use last` record and the harness label constants.
- `src/main.rs` — the single command router; every Herdr action and internal reinjection enters here.
- `src/shell.rs` — `build_command()` / `shell_quote()`; quote every executable path and argument separately.
- `herdr-plugin.toml` — actions and popup panes.
- `config.example.toml` — the executable specification for this plan.

## The specification file

`config.example.toml` at the repository root encodes every schema decision with comments. **Read it before writing any code in this plan.** It is uncommitted and currently still carries a `[[workspaces]]` section that this plan removes.

## Commit & branching style

- Branch: `main`.
- Commit with `/chronicle:commit`. Do not hand-write `git commit`.
- Never commit secrets.
- **Rebuild before release**: `zsh scripts/build.zsh` regenerates the committed `bin/workbench`. A linked checkout still runs that artifact, so a stale binary is the usual cause of code and behaviour disagreeing. Individual tasks do not need to rebuild; the closing review task does.

## Verification baseline

Commands every task can rely on:

- `cargo test` — the whole suite.
- `cargo test config::` / `cargo test flows::agent::` — one module.
- `cargo clippy -- -D warnings` — must be clean.
- `zsh scripts/build.zsh` — rebuild the committed binary (closing review task only).

Sub-agents never open a live Herdr popup. Layout correctness is proven with `FakeClient` socket-sequence assertions instead.

## Decisions frozen during interview

- **`[[workspaces]]` is out of scope** — removed from `config.example.toml`. `dashboard_workspace` keeps its current meaning: a literal Herdr workspace label matched against `workspace.list`. `src/flows/dashboard.rs` is not touched by any task.
- **Non-default layouts are reached with `--layout <name>`** — plus one `herdr-plugin.toml` action per layout. No flag falls back to `default_tab_layout`.
- **`workbench config migrate` is deleted entirely** — the subcommand and its whole implementation surface.
- **Five env-var overrides are deleted** — `Q_AGENT_MODEL_ORDER`, `Q_AGENT_MODELS`, `Q_AGENT_MODEL_ARGS`, `Q_CLAUDE_EXTRA_ARGS`, `Q_CODEX_EXTRA_ARGS`. The path scalars and `Q_DASHBOARD_WORKSPACE` stay.
- **Built-in defaults reproduce today's behaviour exactly** — three agents, four claude options, the three-pane `agentic-coding` layout.
- **A user-written section replaces the built-in section whole** — no merge. Writing one `[[agents]]` entry yields exactly one harness option.
- **All validation happens at `Config::load()`**, before the first socket call. `create_popup_tab` closes its tab on failure, but the in-pane path `apply_launch_layout` has no cleanup, so a half-built layout would be left on screen.
- **`ratio` is each pane's own share**; the loader converts with `herdr = 1 - ratio`. Confirmed on screen: the agent pane is the narrow one (~38%).
- **The `use last` record stores stable ids** — agent name, option name, layout name — and `STATE_VERSION` becomes `2`.
- **This is a breaking change with no migration path.** Declared in the CHANGELOG.

## Herdr protocol facts you will need

- Open a fresh connection per request. Reuse hangs.
- `pane.split` accepts `direction` of `"right"` or `"down"` **only**. `left` and `up` exist for focus/resize/swap/neighbor, not for splitting.
- `pane.split` takes `target_pane_id`, `direction`, `ratio`, `cwd`, `env`, `focus`, and returns `pane.pane_id`.
- `pane.send_input` takes `pane_id`, `text`, `keys`. Use `"enter"` to submit; `"cr"` is rejected. The text is typed into the pane's interactive shell, so it crosses a shell boundary — build it with `src/shell.rs`.
- `tab.create` takes `label`, `cwd`, `env`, `focus`, and optionally `workspace_id`; it returns both `tab.tab_id` and `root_pane.pane_id`.
- Codex leaves the TTY and Kitty keyboard protocol dirty after termination. The restart reset sequence must stay before reinjection.

## Label rendering

An icon and a label are joined by **exactly two spaces**: `${icon}  ${label}`. This is an existing convention — `src/flows/agent.rs:514` splits a harness label on a two-space separator, and `compose_label()` at `:838` joins usage and branch the same way. A missing `icon` renders the label alone, with no leading spaces. An agent's `label` defaults to its `name`.

Today's constants, which the built-in defaults must reproduce byte for byte:

```rust
// src/state.rs
pub const HARNESS_CLAUDE: &str = "\u{f15ce}  claude code";
pub const HARNESS_CODEX: &str = "\u{ee0d}  codex";
pub const HARNESS_OPENCODE: &str = "\u{f169f}  opencode";

// src/flows/agent.rs
const FILES_LABEL: &str = "\u{f0968}  Files";
const TERM_LABEL: &str = "\u{f489}  term";
const AGENT_LABEL: &str = "\u{f169f}  agent";
```

**Never type or paste a Nerd Font glyph into a Rust source file.** The `Edit` tool silently drops plane-15 codepoints, and a bash heredoc corrupts them by losing the fifth hex digit. Write `"\u{f15ce}"` escapes in Rust. When a glyph must reach a TOML file, use the repo's `unicode-edit` skill.

## Today's behaviour, as a baseline

The six launch argv lines the built-in defaults must still produce:

```
claude --model claude-opus-4-8
claude --model opusplan --effort medium
ccr code
claude --model claude-fable-5
codex
opencode
```

The **five** socket calls `build_side_panes()` makes today, in order:

1. `pane.split` — `target_pane_id` = agent pane, `direction: "right"`, `ratio: 0.38`, `cwd`, `env: {"Q_NO_BANNER": "1"}`, `focus: false`
2. `pane.rename` — the new pane, `label: "\u{f0968}  Files"`
3. `pane.send_input` — the new pane, `text: "yazi ."`, `keys: ["enter"]`
4. `pane.split` — `target_pane_id` = files pane, `direction: "down"`, `ratio: 0.9`, `cwd`, `focus: false` (no `env`)
5. `pane.rename` — the new pane, `label: "\u{f489}  term"`

Five is the parity number used everywhere in this plan. `build_side_panes` never touches the root pane: the root's rename and its `env` belong to the callers (`create_popup_tab` passes `env` to `tab.create`; `apply_launch_layout` renames the root). Counting that caller-owned rename would make it six — do not.

An empty `pane_id` back from `pane.split` is an error, not a silent skip.
