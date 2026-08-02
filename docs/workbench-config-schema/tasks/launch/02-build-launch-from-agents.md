# LAUNCH-02: Build launch argv from agents

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: config/04
> **Blocks**: launch/01, wiring/03, wiring/04
> **Status**: todo

## Goal

`build_launch()` assembles its argv from an `[[agents]]` entry and one of its options, so the three hardcoded harness branches and the `CCR` escape disappear while every existing launch line stays byte-identical.

## Files to create / modify

- `src/flows/agent.rs` (modify) — `build_launch`, its call site, and the launch tests.

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

### New signature

```rust
fn build_launch(config: &Config, agent_name: &str, option_name: Option<&str>) -> Result<Vec<String>>
```

Both parameters are stable config **names**, not rendered labels.

### Assembly order — exactly three steps

1. **Base command.** The option's `command` when the option sets one, else the agent's `command`.
2. **Append the option's `args`.**
3. **Append the agent's `extra_args`.**

That is the whole function. Nothing branches on which agent it is.

### What gets deleted

Three hardcoded branches in the current body at `src/flows/agent.rs:616-643` all go:

```rust
if harness.contains("codex") { ... }        // delete
if harness.contains("opencode") { ... }     // delete
if model == "CCR" { ... }                   // delete — lines 631-635
```

**CCR becomes an ordinary option.** Its `command` override supplies `["ccr", "code"]`. Because CCR's `args` list is empty and `extra_args` belongs to the agent rather than the option, the assembly rule alone reproduces today's special case exactly: `ccr code`, with no model flag. The comment currently justifying the escape ("CCR is not a model") is deleted with it — the config now says the same thing structurally.

Note the asymmetry this creates and leave it alone: an option's `command` override **replaces the executable** but does **not** suppress the agent's `extra_args`. In the shipped default config claude's `extra_args` is empty, so CCR's argv is unchanged. If a user later adds claude `extra_args`, CCR inherits them — that is the documented meaning of "appended to every launch of this agent" in `config.example.toml`, and it is the intended behaviour, not a bug to work around.

### Errors

`config.agent(agent_name)` and the option lookup are both guaranteed to resolve for a validated config, but this function also serves the restart path where a stored name is replayed. Keep both lookups fallible and name the offending value in the message, in the house style:

```rust
.with_context(|| format!("no agent entry for: {agent_name}"))
.with_context(|| format!("agent {agent_name} has no option: {option_name}"))
```

An agent with a non-empty `options` list called with `option_name == None` is an error — name it. An agent with an **empty** `options` list called with `option_name == None` is the normal codex/opencode path and must succeed.

### Call site

The single caller is inside the menu flow:

```rust
let launch = build_launch(config, &agent_name, option_name.as_deref())?;
```

The menu flow now yields stable names rather than rendered labels, so no stripping or substring matching is needed at the call site.

### Tests

The parity assertion is the point of this task. Against the built-in default config, assert **byte-equal** argv — compare whole `Vec<String>` values, never lengths or `contains`:

| agent | option | expected argv |
|---|---|---|
| `claude code` | `Opus` | `["claude", "--model", "claude-opus-4-8"]` |
| `claude code` | `OpusPlan (Sonnet)` | `["claude", "--model", "opusplan", "--effort", "medium"]` |
| `claude code` | `CCR` | `["ccr", "code"]` |
| `claude code` | `Fable 5` | `["claude", "--model", "claude-fable-5"]` |
| `codex` | `None` | `["codex"]` |
| `opencode` | `None` | `["opencode"]` |

Then four behavioural tests:

1. **`extra_args` reaches every option.** Give an agent `extra_args = ["--search", "--profile", "work"]` and assert both a plain option and a `command`-override option end with those three entries in that order.
2. **An override still takes its own `args`.** An option with `command = ["ccr", "code"]` **and** `args = ["--flag"]` yields `["ccr", "code", "--flag"]`, proving `args` is appended to the resolved base rather than to the agent's `command`.
3. **A spaced argument survives as one argument.** An option with `args = ["--cd", "/Users/q/My Projects"]` yields a four-element argv whose last element is `/Users/q/My Projects` — not five elements. This is why every argument is its own array entry in the schema.
4. **Missing names are named errors.** An unknown agent name and an unknown option name each produce an error whose message contains the offending value; an agent with options called with `None` errors too.

**The existing tests at `src/flows/agent.rs:1428-1441` drive `config.codex_extra_args` directly** and must be rewritten against `[[agents]]` — that field no longer exists. Preserve their intent: an agent with no extra args yields a bare command; adding extra args extends it; and an extra arg containing a space stays one argument.

## Acceptance criteria

- [ ] `build_launch(config, agent_name, option_name)` takes stable config names and assembles base command → option `args` → agent `extra_args`, in that order.
- [ ] The `harness.contains("codex")`, `harness.contains("opencode")`, and `model == "CCR"` branches are all gone from the file.
- [ ] All six default launch lines are asserted byte-equal against whole `Vec<String>` values.
- [ ] An option's `command` override replaces the executable and still receives that option's own `args`.
- [ ] An agent with an empty `options` list succeeds with `option_name == None`; an agent with options fails with a message naming the agent.
- [ ] An unknown agent name and an unknown option name each error with the offending value in the message.
- [ ] An argument containing a space survives as exactly one argv entry.

## Verification

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `cargo test flows::agent::` — the six-line parity test and the four behavioural tests are present and passing.
- [ ] `grep -n 'model == "CCR"\|contains("codex")\|contains("opencode")' src/flows/agent.rs` returns nothing. Grep for the deleted **conditionals**, not for the string `CCR` — the parity tests in this task legitimately call `build_launch(config, "claude code", Some("CCR"))`, so a blanket `CCR` grep can never pass on a correct implementation.
- [ ] Run `git status --short` and quote it. Expect `src/flows/agent.rs`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Any of the six argv lines differs from today, or a hardcoded harness branch survives | Lines match but assembly order is wrong (e.g. `extra_args` before `args`), so a user-set extra arg lands in the wrong position | All six byte-equal, order is base → `args` → `extra_args`, override replaces the executable only |
| Test coverage | ×2 | Parity asserted with `len()` or `contains` instead of whole-vector equality | Six lines covered, no failure-path or spaced-argument test | Six-line parity plus `extra_args` reach, override-with-args, spaced argument, and three named-error cases |
| Interface & readability | ×1 | Branching on agent name reintroduced under a new shape | Works but the three assembly steps are tangled with the lookups | Lookups first, then three unconditional appends; no per-agent knowledge in the function |
| Assumptions & docs | ×1 | The `extra_args`-still-applies-to-overrides behaviour is silently changed | Behaviour kept, unexplained | A comment records that an override replaces the executable but not `extra_args`, matching the schema's documented meaning |

## Out of scope

- Deciding which agent and option to launch — that is the menu flow's job; this function receives them.
- Quoting argv for a pane's shell — `src/shell.rs` already owns that and is unchanged.
- Adding a per-option `extra_args` — Deferred. No current requirement asks for it; the agent-level list is the only one in the schema.
