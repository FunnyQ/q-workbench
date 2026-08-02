# Workbench config schema redesign

> **Status**: approved
> **Owner**: Q
> **Last updated**: 2026-08-02

## Overview

Replace five flat configuration fields in the `q.workbench` Herdr plugin with two declarative sections — `[[tab_layouts]]` and `[[agents]]` — so pane arrangement, harness list, and model menus become config-driven instead of hardcoded Rust. Breaking change with no migration path.

## Goals

- A user describes a tab's pane arrangement in TOML and the launcher builds it, instead of five hardcoded socket calls.
- A user adds, removes, or reorders harnesses and model options in TOML, without recompiling.
- Omitting a choice in the layout means the launcher asks for it, so a layout that pins nothing reproduces today's popup exactly.
- The `CCR` special case disappears; it becomes an ordinary option with a `command` override.
- Every configuration error is reported by `Config::load()`, before the first socket call.

## Non-goals

- **`[[workspaces]]`.** Dropped from this version. `dashboard_workspace` keeps its current meaning: a literal Herdr workspace label matched against `workspace.list`. `src/flows/dashboard.rs` is not touched.
- **Any old-TOML → new-TOML conversion.** This is a declared breaking change.
- **Keeping `workbench config migrate`.** The whole zsh migration surface is deleted.
- **New split directions.** Herdr's `pane.split` accepts `right` and `down` only.
- **Changing the project picker's workspace behaviour.** Only its `agent::inject` call site changes.
- **Changing `use last` into a configurable entry.** It is runtime state, not configuration.

## Context

`q.workbench` is a Herdr plugin: a Rust binary (`bin/workbench`) registered by `herdr-plugin.toml`. Its agent tab is hardcoded three ways:

- `build_side_panes()` at `src/flows/agent.rs:178-218` makes five socket calls with literal ratios (`0.38`, `0.9`), literal labels, and `Q_NO_BANNER` on the first split only.
- The harness menu at `src/flows/agent.rs:505-509` is a three-element array of `&'static str` constants that live in `src/state.rs:19-21`.
- `build_launch()` at `src/flows/agent.rs:616-643` branches on substring matches against those constants and carries `if model == "CCR"` as a hardcoded escape.

A prior planning session designed the replacement and wrote `config.example.toml` as the executable specification: every schema decision is encoded there with comments. That file is uncommitted and is the primary reference for every task in this plan.

The governing rule is **omission means ask**. A pane omitting `agent` gets the harness menu; omitting `option` gets the model menu; a layout omitting `tab_label` gets the usage menu, whose answer titles both the tab and the agent pane. The example file's `agentic-coding` layout omits all three and must reproduce today's popup byte for byte.

## Requirements

### MVP

1. **Two-section schema** — `[[tab_layouts]]` (with nested `[[tab_layouts.panes]]`) and `[[agents]]` (with nested `[[agents.options]]`) replace `order`, `models`, `model_args`, `claude_extra_args`, `codex_extra_args`.
   - Acceptance: `config.example.toml` parses through `Config::load()` with `deny_unknown_fields` on every struct.
2. **Built-in defaults reproduce today** — no config file yields the three current harnesses, the four current model options, and the current three-pane layout.
   - Acceptance: a test asserts the default `Config` produces the same six launch argv lines and the same socket-call sequence as today.
3. **Whole-section replacement** — a user-written `[[agents]]` replaces the built-in agents entirely; no merge.
   - Acceptance: a config with one `[[agents]]` entry yields exactly one harness menu option.
4. **Load-time validation** — every referential and structural error fails before any socket call.
   - Acceptance: each of the 25 rejection branches in `tasks/_context/validation-matrix.md` has its own test asserting the offending value appears in the message.
5. **Omission means ask** — the three menus read from the resolved layout.
   - Acceptance: the `agentic-coding` layout asks all three menus; the `personal-assistant` layout asks none.
6. **`build_launch` reads `[[agents]]`** — `command` + `option.args` + `extra_args`, with `option.command` overriding `command`.
   - Acceptance: the four claude options, codex, and opencode reassemble to today's exact argv.
7. **`build_side_panes` iterates the pane list** — `split_from`, `direction`, `1 - ratio`, `${icon}  ${label}`, per-pane `env`, `command`.
   - Acceptance: a `FakeClient` assertion reproduces today's five `build_side_panes` calls for the `agentic-coding` layout — two splits, two renames, one send-input. The root pane's own rename and `env` stay with the callers and are outside this count.
8. **`--layout <name>`** reaches a non-default layout from the CLI and from `herdr-plugin.toml`.
   - Acceptance: `agent popup --layout personal-assistant` opens with everything pinned.
9. **State v2** — the pane record stores agent, option, and layout names; `STATE_VERSION` is `2`.
   - Acceptance: a v1 record is dropped, not misread; a restart of a fully-pinned tab asks nothing.

### Later

- **Workspace creation from a predefined list** — the feature `[[workspaces]]` was originally designed for. Deferred; needs its own plan.
- **Empirically measured ratio semantics** — the `1 - ratio` conversion is inferred from today's values. Confirmed visually, but a measured, documented rule would be better.

## Tech decisions

- **Stack**: Rust 2021, `serde` + `toml 0.8` (already in `Cargo.toml`), `anyhow` for errors.
- **Storage**: TOML at `${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench/config.toml`; JSON state at `$HOME/.local/state/herdr-workbench/last-agent.json`.
- **Deployment**: `zsh scripts/build.zsh` writes the committed `bin/workbench` artifact that `herdr-plugin.toml` runs.
- **Conventions**: see `tasks/_context/shared.md`.

### Frozen during the interview

| Decision | Choice |
|---|---|
| `[[workspaces]]` | Out of scope. Removed from `config.example.toml`. |
| Non-default layout | `--layout <name>` flag plus one manifest action per layout. |
| `workbench config migrate` | Deleted entirely. |
| Old env-var overrides | The five model/extra-args variables deleted. Path scalars and `Q_DASHBOARD_WORKSPACE` stay. |
| `use last` record | Stores stable ids (agent name, option name, layout name). `STATE_VERSION` → `2`. |
| Defaults | Reproduce today; whole-section replacement. |
| `ratio` | Confirmed: agent pane is the narrow one. `herdr = 1 - ratio`. |
| Verification | `cargo test` + `cargo clippy -- -D warnings` per task; `FakeClient` socket-sequence assertions for pane work; `zsh scripts/build.zsh` at the final review. |

## Architecture

```
config.toml ──► Config::load()  ── validate() ─┐   (all errors surface here,
                  │                            │    before any socket call)
                  ├── Vec<Agent>               │
                  └── Vec<TabLayout>           │
                            │                  │
    --layout <name> ────────┤                  │
    default_tab_layout ─────┘                  │
                            ▼                  ▼
                    resolve_layout()  ──► choose_agent()  ──► AgentChoice
                            │                  │  harness menu ⇠ omitted `agent`
                            │                  │  model menu   ⇠ omitted `option`
                            │                  │  usage menu   ⇠ omitted `tab_label`
                            │                  └─► build_launch()  ──► argv
                            ▼
                    build_side_panes()  ──► pane.split / rename / send_input
```

The layout is resolved once, early, and carried through the flow. Panes 2..n are built after every menu closes, because splitting sooner resizes the pane the menus are drawing into and the chosen worktree must determine the cwd of every pane.

### Validation rules (all at `Config::load()`)

1. `default_tab_layout` names a known layout.
2. A pane's `agent` names a known agent.
3. A pane's `option` names one of that agent's options.
4. `option` set while `agent` is omitted is an error — an option belongs to exactly one agent.
5. A layout's first pane is its root and must be `type = "agent"`.
6. Exactly one `type = "agent"` pane per layout.
7. Pane names are unique within a layout.
8. `split_from` names an earlier pane in the same layout (this makes cycles unreachable).
9. `direction` is `right` or `down`; required on every non-root pane.
10. `ratio` is strictly between 0 and 1; required on every non-root pane.
11. `command` is required for `type = "command"` and forbidden for the other two types.
12. Layout names are unique; agent names are unique; option names are unique within an agent.

## Bucketing

- **Strategy**: layer, following the dependency direction.
- **Why**: the schema is the contract. Nothing in `launch/` can be written until the types, defaults, and validation exist. Once they do, the argv builder and the pane builder are independent and run in parallel; the menu rewrite follows the argv builder, because it is the caller that hands `build_launch` its new arguments.

### Buckets

- **`config/`** — `src/config.rs` only. Starts immediately, ends when a validated `Config` carrying agents and layouts exists.
- **`launch/`** — `src/flows/agent.rs`. Starts once `config/` closes, ends when menus, launch argv, and pane building all read from the layout.
- **`wiring/`** — the CLI, the manifest, state, docs, and the closing review.

## Task index

| Bucket | NN | Title | Status | Pass line | Depends on |
|---|---|---|---|---|---|
| config | 01 | remove zsh migration | todo | > 4.0 | — |
| config | 02 | schema types | todo | > 4.0 | config/01 |
| config | 03 | built-in defaults | todo | > 4.0 | config/02 |
| config | 04 | load-time validation | todo | > 4.0 | config/03 |
| launch | 02 | build launch from agents | todo | > 4.0 | config/04 |
| launch | 03 | side panes from layout | todo | > 4.0 | config/04 |
| launch | 01 | menus read the layout | todo | > 4.0 | config/04, launch/02 |
| wiring | 01 | layout flag and manifest | todo | > 4.0 | launch/01, launch/03 |
| wiring | 02 | agent state v2 | todo | > 4.0 | launch/01, wiring/01 |
| wiring | 03 | example config and changelog | todo | > 4.0 | launch/02, wiring/01 |
| wiring | 04 | final review 🏁 | todo | > 4.0 | launch/02, wiring/02, wiring/03 |

## Cross-bucket dependencies

```
config/01 → config/02 → config/03 → config/04 ─┬─► launch/02 ─► launch/01 ─┬─► wiring/01 ─┬─► wiring/02 ─┐
                                               │                           │              │             ├─► wiring/04
                                               └─► launch/03 ──────────────┘              └─► wiring/03 ─┘
```

`launch/02` and `launch/03` are the parallel window. `launch/01` waits on `launch/02` because the menu flow is what calls `build_launch(config, agent_name, option_name)` — the signature that task introduces. Everything else is a chain.

## Open questions

1. **Workspace creation from a predefined list** — the feature `[[workspaces]]` was designed for. Q wants it, but it is a separate plan. Nothing here blocks on it.
2. **Ratio semantics are inferred, not measured** — `herdr = 1 - ratio` reproduces today's values and matches what Q sees on screen, but Herdr does not document the field. Worth measuring against a live session during or after execution.

## Known gaps

- The pane label used by `agent inject` before any menu runs is an assumption, not a confirmed decision: the layout's root-pane `label` when the layout pins one, else today's `AGENT_LABEL` constant. Recorded in the CLI wiring task.
- **The compatibility bridge lives inside the schema task, not in one of its own.** Review flagged this as mixed concerns: the schema task also rewrites the model menu's option list, the `build_launch` body, and the claude arm of `last_choice_is_valid`, which later tasks then rewrite again. Kept deliberately. The bridge exists solely so the schema task's own `cargo test` gate can run — deleting five fields breaks three call sites, and nothing else makes them compile. It is one method plus three inlined lookups, fully specified in the task, and each later task deletes its own share. A separate task would move the same twenty lines and add a dependency edge without removing any work. Do not "fix" this by splitting it.
- **The validation matrix is duplicated between `_context/validation-matrix.md` and the implementing task's body.** Intentional: `_context/` is the single source, and the task inlines it so an executor never has to cross-reference mid-implementation. If a branch changes, change the context file first.

## References

- `config.example.toml` — the executable specification for this plan.
- `.handoffs/2026-08-02-1745-session.md` — the design session that produced it.
- `~/.claude/plugins/cache/q-lab-marketplace/herdr/0.2.3/skills/herdr/references/cli.md:91` — `pane.split` accepts `right` and `down` only.
