# LAUNCH-01: Menus read the layout

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: config/04, launch/02
> **Blocks**: wiring/01, wiring/02
> **Status**: done

## Goal

The harness, model, and usage menus are driven by the resolved tab layout, so a pinned choice skips its menu and an omitted choice asks — making a layout that pins nothing reproduce today's popup exactly.

## Files to create / modify

- `src/flows/agent.rs` (modify) — `choose_agent`, `choose_agent_with_last`, `choose_agent_with`, `AgentChoice`, the `MODEL_TITLE` usage, and the module's menu tests.

## Implementation notes

### What already exists

The config loader is finished and validated. These types are available and every reference in a loaded `Config` is guaranteed to resolve — an executor **must not** re-validate them:

```rust
pub struct TabLayout { pub name: String, pub tab_label: Option<String>, pub panes: Vec<LayoutPane> }
pub struct LayoutPane {
    pub name: String, pub label: Option<String>, pub icon: Option<String>,
    pub pane_type: PaneType, pub agent: Option<String>, pub option_name: Option<String>,   // TOML key: option
    pub command: Option<String>, pub direction: Option<Direction>, pub ratio: Option<f64>,
    pub split_from: Option<String>, pub env: BTreeMap<String, String>,
}
pub enum PaneType { Agent, Command, Shell }
pub enum Direction { Right, Down }
pub struct Agent { pub name: String, pub label: Option<String>, pub icon: Option<String>,
    pub command: Vec<String>, pub extra_args: Vec<String>, pub options: Vec<AgentOption> }
pub struct AgentOption { pub name: String, pub args: Vec<String>, pub command: Option<Vec<String>> }

// on Config
pub fn layout(&self, name: &str) -> Option<&TabLayout>
pub fn agent(&self, name: &str) -> Option<&Agent>
pub fn render_label(icon: Option<&str>, label: &str) -> String  // "{icon}  {label}", exactly two spaces
```

**Field naming.** Throughout this file, `option` names the **TOML key**; the Rust field is `option_name`, declared `#[serde(rename = "option")]` because bare `Option` reads as the standard-library type at every use site. Write `root.option_name` in code and `option` in prose.

The launch-argv builder is also already finished, with this exact signature — call it, do not rewrite it:

```rust
fn build_launch(config: &Config, agent_name: &str, option_name: Option<&str>) -> Result<Vec<String>>
```

It assembles the option's `command` override (else the agent's `command`), then the option's `args`, then the agent's `extra_args`. An agent with an empty `options` list succeeds with `option_name == None`.

Guaranteed by load-time validation: the layout's **first pane is its root and is `PaneType::Agent`**; a pane's `agent` names a known agent; a pane's `option` names one of that agent's options; `option` set without `agent` was already rejected. So inside this flow, `config.agent(name).expect(...)` on a validated reference is legitimate — prefer a plain `.expect("validated at load")` over an error path the code cannot reach.

### The governing rule: omission means ask

The layout's root pane is `layout.panes[0]`. Three independent decisions:

| Config | Omitted → | Pinned → |
|---|---|---|
| root pane `agent` | run the harness menu | skip it, use the named agent |
| root pane `option` | run the model menu | skip it, use the named option |
| layout `tab_label` | run the usage menu | skip it, use the string verbatim |

A layout that omits all three asks all three questions, in the order harness → model → usage. That is today's popup, unchanged.

### Harness menu

Built from `config.agents` **in config order**. Each entry's menu text is `render_label(agent.icon.as_deref(), agent.label.as_deref().unwrap_or(&agent.name))`. The menu returns a padded, rendered string; `strip_pad` removes the centering pad, and the result is mapped back to the agent by comparing against the same rendered label. Keep the existing `menu.choose(HARNESS_TITLE, "Choose a harness.", &options, 8)` call shape.

### Model menu

Built from the chosen agent's `options`, **in config order**, using each option's `name` **verbatim** as the menu text — options carry no icon.

**An agent whose `options` list is empty skips the model menu entirely.** That is how codex and opencode behave today. This replaces the substring test at `src/flows/agent.rs:545`:

```rust
} else if harness.contains("claude code") {
```

Delete that branch. The condition becomes "does the chosen agent have any options", not "is this agent claude".

**Behaviour change worth its own test:** `MODEL_TITLE` is currently the literal constant `"\u{f09d1}  claude code"` — hardcoded to claude even though it titles a generic menu. It must become the **chosen agent's rendered label**, so picking codex (if codex ever gains options) titles the model menu with codex. Remove the `MODEL_TITLE` constant and pass the rendered agent label instead. Do not paste a Nerd Font glyph anywhere; the label comes from config at runtime.

### Usage menu and the label

`select_usage()` is unchanged. The precedence for the tab/pane label is:

1. `fixed_usage` — the restart path passes the pane's current label; the project picker passes its pinned tab label. Wins over everything.
2. The layout's `tab_label` when set — used verbatim.
3. The usage menu.

In all three cases `compose_label(&usage, branch.as_deref())` still runs, so a chosen worktree branch is appended after exactly two spaces.

### Signature changes

`choose_agent`, `choose_agent_with_last`, and the `#[cfg(test)]` shim `choose_agent_with` all gain a `layout: &TabLayout` parameter. Place it next to `config` — the two are read together throughout.

```rust
pub fn choose_agent(
    config: &Config,
    layout: &TabLayout,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    cols: u16,
    lines: u16,
    last: Option<(String, Option<String>)>,
) -> Result<Option<AgentChoice>>
```

### `AgentChoice` now carries stable names

Today `AgentChoice.harness` holds the pad-stripped **rendered** harness label and `model_label` holds the rendered model label. Both change to the stable config ids, because the `use last` state record keys on them and a renamed icon must not invalidate a saved choice:

```rust
pub struct AgentChoice {
    /// The pane and tab label: the usage label, plus two spaces and the branch
    /// when a worktree was chosen.
    pub label: String,
    pub project_dir: PathBuf,
    pub branch: Option<String>,
    /// argv, ready for `exec` or for a pane command.
    pub launch: Vec<String>,
    /// The `[[agents]]` entry's `name`, not its rendered label.
    pub agent_name: String,
    /// The chosen `[[agents.options]]` entry's `name`; `None` for an agent with no options.
    pub option_name: Option<String>,
}
```

`label` keeps carrying what is shown. Callers that pass `choice.harness` / `choice.model_label` into `state::write_state` now pass `choice.agent_name` / `choice.option_name`.

### The `use last` entry

The entry at `src/flows/agent.rs:510-520` keeps its visible shape:

```
use last: {agent label}          // agent with no options
use last: {agent label} · {option name}
```

**The state record is NOT rewritten in this task.** `src/state.rs` still stores v1 records and still owns whatever validity rule it currently has. Do not change its schema, its version, or `last_choice_is_valid` here — a separate, later piece of work bumps the record to v2, switches it to stable ids, and adds the stale-record tests. Trying to do it here means editing a file this task does not own and cannot test properly.

What this task must do, and no more:

- Keep the `use last` entry rendering and short-circuiting exactly as it does today. Selecting it still skips both the harness and the model menu.
- Feed it whatever `state::get_for_pane` currently returns, through the existing `last_choice_is_valid` filter, with the existing call shape.
- At the point where the selected `use last` value becomes an `AgentChoice`, convert it to `agent_name` / `option_name`. Because the stored value is still a rendered label at this stage, resolve it by matching against each agent's rendered label — the same mapping the harness menu already needs. Mark that conversion with a comment saying it is a bridge until the record stores names directly.
- Leave the "stored agent no longer exists" behaviour to whatever the current filter does. It is specified and tested by the state rewrite, not here.

This keeps the task independently green: `cargo test` passes against an unchanged `src/state.rs`.

### Tests

Use the existing `FakeMenu` harness in this module (it returns a queued sequence of `Option<String>` answers and records the prompts it was asked). Build layouts inline in each test rather than reading `config.example.toml`.

1. **Three menus, in order.** A layout with no `agent`, no `option`, no `tab_label` drives exactly three prompts: harness, then model, then usage. Assert the count is 3 and assert the recorded titles in order.
2. **Zero menus.** A layout pinning `agent = "claude code"`, `option = "Opus"`, `tab_label = "Personal Assistant"` drives **zero** prompts and yields `agent_name == "claude code"`, `option_name == Some("Opus")`, `label == "Personal Assistant"`.
3. **Empty options skips the model menu.** An agent with `options: vec![]`, chosen from the harness menu, drives exactly two prompts (harness, usage) and yields `option_name == None`.
4. **Model menu title follows the agent.** The recorded model-menu title equals the chosen agent's rendered label, not a claude literal.
5. **Cancellation is clean.** Cancelling at the harness menu, at the model menu, and at the usage menu each returns `Ok(None)`. No worktree is created and no `AgentChoice` is produced — this module decides, it never acts.

## Acceptance criteria

- [x] `choose_agent`, `choose_agent_with_last`, and `choose_agent_with` take a `&TabLayout` and read the root pane's `agent` / `option` and the layout's `tab_label` from it.
- [x] A pinned `agent` skips the harness menu; a pinned `option` skips the model menu; a pinned `tab_label` skips the usage menu.
- [x] An agent with an empty `options` list skips the model menu, and the `harness.contains("claude code")` substring test is gone from the file.
- [x] The model menu's title is the chosen agent's rendered label; the `MODEL_TITLE` constant is removed.
- [x] `AgentChoice` exposes `agent_name: String` and `option_name: Option<String>` holding config names, and no longer exposes rendered labels under `harness` / `model_label`.
- [x] `fixed_usage` still takes precedence over `tab_label`, which takes precedence over the usage menu; `compose_label` still appends the branch after two spaces.
- [x] The `use last` entry still renders and still short-circuits both menus, and `src/state.rs` is unchanged by this task — its schema, version, and validity rule are somebody else's work.
- [x] Cancelling at any of the three menus returns `Ok(None)` and creates nothing.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo test flows::agent::` — the five tests above are present and passing.
- [x] `grep -n 'MODEL_TITLE\|contains("claude code")' src/flows/agent.rs` returns nothing.
- [x] Run `git status --short` and quote it. Expect `src/flows/agent.rs`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A pinned field still opens its menu, or the substring test on `"claude code"` survives | Pinning works but menu order drifts, or `fixed_usage` no longer beats `tab_label` | All three omission rules hold, precedence is `fixed_usage` → `tab_label` → menu, empty `options` skips the model menu |
| Test coverage | ×2 | No test asserts prompt counts | Only the all-asked path is tested | All five tests present: three-menu order, zero-menu pin, empty-options skip, model title follows agent, cancellation at each menu |
| Interface & readability | ×1 | `layout` threaded through as loose fields, or validity re-checked inline | Signature is right but the menu construction is duplicated per menu | `&TabLayout` sits beside `config`, rendered-label mapping lives in one place, validated references use `.expect("validated at load")` rather than dead error paths |
| Assumptions & docs | ×1 | The `MODEL_TITLE` behaviour change lands silently | Change made but unexplained | A comment records why `AgentChoice` stores names rather than labels, and why the model menu title now follows the agent |

## Out of scope

- **Touching `src/state.rs` at all** — Deferred. The record's schema, its `STATE_VERSION`, `last_choice_is_valid`, and the stale-record behaviour are rewritten together in a later piece of work. Keep the existing call shape and bridge the rendered label to a name at the `AgentChoice` boundary. If this task edits `src/state.rs`, that is a scope violation.
- Threading a `--layout` flag through the CLI — Deferred. This task takes a `&TabLayout` as a parameter; resolving which layout to pass is separate work.
- Changing `select_usage`, `select_worktree`, `realise_worktree`, or `compose_label` — they are unchanged.
