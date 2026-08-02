# WIRING-02: Agent state v2 keyed on stable ids

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: launch/01, wiring/01
> **Blocks**: wiring/04
> **Status**: todo

## Goal

The `use last` record stores stable configuration ids instead of rendered menu labels, and remembers which layout a pane was launched with, so restarting a fully-pinned tab asks nothing.

## Files to create / modify

- `src/state.rs` (modify) — new record shape, `STATE_VERSION` = 2, validity rule rewritten against config, harness label constants deleted.
- `src/flows/agent.rs` (modify) — write the new record fields; drop the three deleted constant imports; **delete the bridge**. The menu rewrite left a marked bridge that maps a stored *rendered label* back to an agent name, because the record still held labels at that point. Once the record holds names, that mapping is dead — the `use last` entry resolves the stored name through the config to build its display text, and the reverse lookup goes away. Grep `src/flows/agent.rs` for the bridge comment and remove it.
- `src/flows/restart.rs` (modify) — `injected_command()` emits `--layout` from the stored record.

## Implementation notes

### What already exists when this task starts

The config loader exposes agents and layouts by name, and the launch flow already carries stable names on its decision struct. Inline signatures:

```rust
pub struct Agent {
    pub name: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub command: Vec<String>,
    pub extra_args: Vec<String>,
    pub options: Vec<AgentOption>,
}
pub struct AgentOption { pub name: String, pub args: Vec<String>, pub command: Option<Vec<String>> }
pub struct TabLayout { pub name: String, pub tab_label: Option<String>, pub panes: Vec<LayoutPane> }

// on Config
pub fn agent(&self, name: &str) -> Option<&Agent>
pub fn layout(&self, name: &str) -> Option<&TabLayout>

// the resolved decision the launch flow produces
pub struct AgentChoice {
    pub label: String,
    pub project_dir: PathBuf,
    pub branch: Option<String>,
    pub launch: Vec<String>,
    pub agent_name: String,           // the [[agents]] name
    pub option_name: Option<String>,  // the [[agents.options]] name
}
```

`LaunchOptions` and `InjectOptions` each carry `layout: Option<String>`, where `None` means `default_tab_layout`.

### The record

`src/state.rs` currently stores rendered menu labels. Replace the record with stable ids:

```rust
pub struct LastAgentRecord {
    pub agent: String,           // the [[agents]] name
    pub option: Option<String>,  // the [[agents.options]] name
    pub layout: String,          // the [[tab_layouts]] name
    pub recorded_at: u64,
}
```

Keep `#[serde(skip_serializing_if = "Option::is_none")]` on `option`, matching how the current `model` field is written.

Why names and not labels: a record keyed on a rendered label silently invalidates every saved choice the moment an icon changes. A record keyed on `name` only invalidates when the user actually renames the thing, which is the correct trigger.

`layout` is not optional. Every launch resolves to exactly one layout, so there is always a name to store.

### Version bump

`STATE_VERSION` becomes `2`. `read_state()` already filters on `state.version == STATE_VERSION` and falls back to `LastAgentState::default()`, so a v1 file is discarded wholesale rather than deserialized into the wrong shape. Confirm that path still holds after the struct change and add a test for it — this is the whole reason the bump exists.

### Delete the harness constants

These three, currently at `src/state.rs:19-21`, are now config data and must go:

```rust
pub const HARNESS_CLAUDE: &str = "\u{f15ce}  claude code";
pub const HARNESS_CODEX: &str = "\u{ee0d}  codex";
pub const HARNESS_OPENCODE: &str = "\u{f169f}  opencode";
```

Every `use crate::state::{HARNESS_CLAUDE, ...}` import in `src/flows/agent.rs` goes with them, along with any test that references them. Their glyphs survive as the built-in default agents' `icon` values in the config loader, so nothing is lost.

### Validity rule

Today's rule special-cases claude as the only harness that has models. The new rule is general — an agent has options or it does not:

```rust
pub fn last_choice_is_valid(record: &LastAgentRecord, config: &Config) -> bool
```

It returns `true` only when all three hold:

1. `config.agent(&record.agent)` resolves.
2. If that agent's `options` list is non-empty, `record.option` is `Some(name)` and that name appears in the list. If the list is empty, `record.option` is `None`.
3. `config.layout(&record.layout)` resolves.

The `iff` in rule 2 matters in both directions. A stored option for an agent that no longer has any options is stale, and a missing option for an agent that does have them cannot be replayed.

### Reads and writes

- `write_state()` gains the layout name in its signature and writes all three fields. Keep the existing pruning: it lists live panes over the socket and drops records for panes Herdr no longer reports, so a long-lived state file does not accumulate dead entries.
- `get_for_pane()` returns the whole `LastAgentRecord` rather than a tuple. When the record fails `last_choice_is_valid`, it is removed and the file is rewritten atomically through `write_json_atomically`, exactly as today.

Update the harness menu's `use last` entry to render from the record: look up the agent to get its display label, and append the option name when there is one. The existing separator convention is `" · "` between harness and model, after the `use last: ` prefix.

### Restart reinjection

`injected_command()` in `src/flows/restart.rs` (currently lines 220-238) builds the string typed back into the pane. It emits today:

```
agent launch <pane> --usage <label> --no-layout --restart
```

It must also emit `--layout <name>` read from the stored record for that pane. When no record exists, omit the flag entirely and let the launcher fall back to `default_tab_layout`.

The payoff: a tab opened with a fully-pinned layout restarts with no menus at all. Without this, a restart falls back to the default layout and pops a harness menu the layout says should never appear.

`restart_worker` runs in a detached process and can read the state file directly. It does not need a Herdr round trip to learn the layout.

Two things that must not change:

- **`TTY_RESET` stays in front of the launcher** in the returned string. Codex leaves the TTY in raw mode with the Kitty keyboard protocol enabled; without the reset, `gum` renders in the wrong column and ignores arrow keys.
- **`--no-layout` stays.** Restart deliberately does not rebuild the side panes — they survived the kill and rebuilding would duplicate them. `--layout` and `--no-layout` are orthogonal: the first says *which* layout describes this tab, the second says *do not build panes from it*.

Keep quoting every argument separately through `build_command()` from `src/shell.rs`. A layout name with a space must not split into two arguments.

## Acceptance criteria

- [ ] `LastAgentRecord` carries `agent`, `option`, `layout`, and `recorded_at`, with `option` skipped when `None`.
- [ ] `STATE_VERSION` is `2`, and a v1 state file yields the default state rather than an error or a partial read.
- [ ] The three harness label constants and every import of them are gone from the crate.
- [ ] `last_choice_is_valid` enforces all three rules, including the both-directions option rule.
- [ ] `write_state()` records the layout name; `get_for_pane()` returns the record and prunes an invalid one.
- [ ] `injected_command()` emits `--layout <name>` when a record exists and omits it when none does, with `TTY_RESET` still leading and `--no-layout` still present.
- [ ] Every argument in the injected command is quoted separately through `build_command()`.

## Verification

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] A test writes a `{"version":1,...}` file and asserts `read_state()` returns the default.
- [ ] A test stores a record whose agent name is absent from the config, asserts the lookup returns `None`, and asserts the file was rewritten without that pane.
- [ ] A test stores a record whose option name is absent from that agent's options and asserts it is dropped.
- [ ] A test stores a record with no option for an agent that *has* options and asserts it is dropped.
- [ ] A test asserts the injected command for a pane whose record names the pinned layout contains `--layout personal-assistant`, and that the string still starts with the TTY reset sequence.
- [ ] A test asserts the injected command for a pane with no record contains no `--layout`.
- [ ] `rg 'HARNESS_CLAUDE|HARNESS_CODEX|HARNESS_OPENCODE' src/` returns nothing.
- [ ] Run `git status --short` and quote it. Expect `src/state.rs`, `src/flows/agent.rs`, `src/flows/restart.rs`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A v1 record is misread instead of dropped, or restart loses the layout | Record shape correct but the option rule only checks one direction, or `TTY_RESET` / `--no-layout` disturbed | All three validity rules hold both ways, restart replays the stored layout, reset and no-layout intact |
| Test coverage | ×2 | No v1-discard test | Happy path only; stale records untested | v1 discard, stale agent, stale option, missing-option-for-optioned-agent, and both restart paths all covered |
| Interface & readability | ×1 | Labels still leak into the record, or `get_for_pane` returns a widening tuple | Record is stable-id based but the validity rule is inlined at each call site | One `last_choice_is_valid` used by both readers, record returned whole |
| Assumptions & docs | ×1 | The version bump is unexplained | Bump noted without saying what it protects | A comment states that v2 exists so v1 records are dropped rather than misread |

## Out of scope

- Migrating v1 records into v2 — Deferred. The record is a convenience cache, not user data; discarding it costs one menu pass and is declared in the release notes.
- Making `use last` a configurable menu entry — Deferred. It is runtime state, not configuration.
- Storing the chosen worktree branch in the record — Deferred. Restart reuses the pane's existing label as its fixed usage, so the branch is already carried.
