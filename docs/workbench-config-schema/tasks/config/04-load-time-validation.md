# CONFIG-04: Load-time validation

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
> - `../_context/validation-matrix.md`
>
> **Depends on**: config/03
> **Blocks**: launch/01, launch/02, launch/03
> **Status**: done

## Goal

Every referential and structural error in a config is reported by `Config::load()`, with the offending value named in the message, so no launch flow can ever start building a tab it cannot finish.

## Files to create / modify

- `src/config.rs` (modify) — rewrite `Config::validate()` and add one test per rule.

## Implementation notes

### Why this lives at load time

Two entry points build a tab. `create_popup_tab` creates the tab first and closes it on any failure, so a mid-build error is cleaned up. The in-pane path `apply_launch_layout` has **no cleanup** — it splits panes into a tab that already exists and is already on screen. A layout that fails halfway there leaves a half-built tab the user has to dismantle by hand.

So validation cannot be spread across the flow. It runs once, in `Config::load()`, before the first socket call, and a config that loads is a config that can be built.

### House style for the messages

Every message names the offending value. The existing style is the model:

```rust
bail!("model order label has no model entry: {label}");
```

Where a name could be ambiguous across layouts, qualify it: `"layout 'agentic-coding' pane 'files': ..."`. A user staring at a 200-line TOML needs to know which entry to look at.

Validate layouts in file order and panes in list order, so the first error reported is the first error in the file.

### The rules

**1. `default_tab_layout` names a known layout.**

```
default_tab_layout names no tab layout: {name}
```

**2. A pane's `agent` names a known agent.**

```
layout '{layout}' pane '{pane}': agent names no agent entry: {agent}
```

**3. A pane's `option` names one of that agent's options.**

```
layout '{layout}' pane '{pane}': agent '{agent}' has no option: {option}
```

**4. `option` set while `agent` is omitted.**

```
layout '{layout}' pane '{pane}': option requires agent, because an option belongs to one agent: {option}
```

An option name is only meaningful inside one agent's list, so an option without an agent cannot be resolved. Omitting both is fine — that is the "ask for it" case.

**5a. A layout must declare at least one pane.**

```
layout '{layout}': declares no panes; the first pane is the tab root
```

`panes` deserializes with `#[serde(default)]`, so `[[tab_layouts]]` with only a `name` is a valid TOML shape that produces an empty `Vec`. Every rule below indexes `panes[0]`. **Check this first**, before any other per-layout rule, so the root access can never panic and an empty layout fails with a named error instead of an index panic. Its test asserts the message names the layout.

**5. A layout's first pane must be `type = "agent"`.**

```
layout '{layout}': the first pane is the tab root and must be type = "agent", found {type}
```

Stronger than "there is an agent pane somewhere": the position is fixed. Restart-in-place terminates the foreground harness and reinjects a launcher into the surviving shell, and the panes that survive are the ones that were split off the agent pane. The injected launcher ends with `exec`, so the harness replaces the launcher process rather than running as its child. If the agent is not the root, the side panes are children of something the restart destroys.

**6. Exactly one `type = "agent"` pane per layout.**

```
layout '{layout}': exactly one pane may be type = "agent", found {count}
```

A layout with zero panes is caught by the non-empty rule above; a layout whose only agent pane is not the root is caught by the root-type rule. A layout with two agent panes is caught here — name the second pane in the message if it is cheap to do so.

**7. Pane names are unique within a layout.**

```
layout '{layout}': duplicate pane name: {name}
```

`split_from` resolves by name, so a duplicate makes the reference ambiguous.

**8. `split_from` names an earlier pane in the same layout.**

```
layout '{layout}' pane '{pane}': split_from names no earlier pane: {target}
```

"Earlier" is the whole rule. A pane can only split something that already exists when it is created, and panes are created top to bottom. Enforcing earlier-only means a forward reference, a self reference, and a cycle are all the same error — **there is no cycle to detect, so do not build a graph walker.** Checking membership in the set of names seen so far is sufficient and complete.

The root pane must not set `split_from`; it splits nothing.

```
layout '{layout}': the root pane '{pane}' cannot set split_from
```

**9. `direction` is required on every non-root pane and forbidden on the root.**

```
layout '{layout}' pane '{pane}': direction is required for a pane that splits
layout '{layout}': the root pane '{pane}' cannot set direction
```

The enum already restricts the value to `right` or `down` at deserialization time — Herdr's `pane.split` accepts no others — so there is nothing to check here beyond presence.

**10. `ratio` is required on every non-root pane, forbidden on the root, and strictly inside `(0.0, 1.0)`.**

```
layout '{layout}' pane '{pane}': ratio is required for a pane that splits
layout '{layout}': the root pane '{pane}' cannot set ratio
layout '{layout}' pane '{pane}': ratio must be between 0 and 1, exclusive: {ratio}
```

Exclusive at both ends. `0.0` and `1.0` both describe a pane with no area, which Herdr cannot draw. Reject `NaN` through the same check — a comparison against `NaN` is false, so writing the guard as `!(ratio > 0.0 && ratio < 1.0)` catches it for free. Note that in a comment; the inverted form looks like a clippy target otherwise.

**11. `command` is required for `type = "command"` and forbidden for the other two types.**

```
layout '{layout}' pane '{pane}': type = "command" requires command
layout '{layout}' pane '{pane}': command is only valid for type = "command"
```

A `shell` pane runs nothing; an `agent` pane's command comes from the resolved agent.

**11b. A command must actually name something to run.**

```
layout '{layout}' pane '{pane}': command is empty
agent '{agent}': command is empty
agent '{agent}' option '{option}': command override is empty
```

`Vec<String>` and `String` both deserialize happily from `[]` and `""`. An empty agent `command`, an empty option `command` override, or a blank pane `command` line all pass every rule above and then produce an empty argv — which fails inside the launch flow, after the tab is already on screen. That is exactly the failure this whole task exists to prevent, so catch all three here.

Reject a pane `command` that is empty **or whitespace-only** (`command.trim().is_empty()`); a blank line typed into a shell runs nothing. For the two argv arrays, an empty `Vec` is the whole test — do not inspect the individual arguments.

**12. Names are unique across their scope.**

```
duplicate tab layout name: {name}
duplicate agent name: {name}
agent '{agent}': duplicate option name: {name}
```

All three resolve by name, so a duplicate silently shadows.

### Shape

Keep it a plain nested loop over layouts and panes with a `BTreeSet` of seen names. Do not build a visitor, a rule registry, or a trait. It is one function that reads top to bottom in the same order the user reads their file.

```rust
impl Config {
    fn validate(&self) -> Result<()> {
        // rules 12 and 1 first: they are cheap and their failure makes the rest meaningless
        // then, per layout: 5a before anything that indexes panes[0], then 5, 6, 7, 8, 9, 10, 11
        // then, per pane: rules 2, 3, 4
        Ok(())
    }
}
```

### Tests

**Every rule gets its own test, and each asserts the offending value appears in the message.** A test that only asserts `is_err()` does not prove the user can find the problem.

```rust
#[test]
fn a_pane_option_without_an_agent_is_a_named_error() {
    let environment = TestEnvironment::new();
    environment.write(
        r#"
[[tab_layouts]]
name = "solo"
  [[tab_layouts.panes]]
  name = "agent"
  type = "agent"
  option = "Opus"
"#,
    );

    let error = Config::load().expect_err("reject an option without an agent");

    assert!(error.to_string().contains("option requires agent"), "{error}");
    assert!(error.to_string().contains("Opus"), "{error}");
}
```

Write the other eleven the same way. Reuse the existing `TestEnvironment` harness — it already isolates `HOME`, points `Q_WORKBENCH_LOCAL_CONFIG` at a temporary file, and cleans up on drop.

Add two positive tests as well:

- The built-in defaults validate. `Config::load()` with no file must return `Ok`.
- The real `config.example.toml` validates. Write its contents into the test environment's config path and assert `Config::load()` succeeds. This is what makes the example file an executable specification rather than a document: a schema change that the example does not satisfy fails the suite.

Note that the example file's `personal-assistant` layout pins `agent = "claude code"` and `option = "Opus"`, so this test also exercises rules 2 and 3 on the happy path.

### Two error stages, two different contracts

Not every bad config reaches `validate()`. An unknown field, an unknown enum variant (`direction = "left"`), or a wrong type fails inside serde, during `toml::from_str`, before any `Config` exists to inspect. Those errors carry no layout or pane name — and they do not need to. `toml` reports the file path (already added by the existing `with_context` in `Config::load()`) plus the exact line and column of the offending value, which points at the problem more precisely than a name would.

**Do not write a custom `Deserialize` impl to inject names into parse errors.** That is a large amount of hand-written deserialization code bought for a message that is already actionable. The name-the-value contract below applies to validation-stage errors only.

Add one test proving the parse stage is actionable: a config with `direction = "left"` fails, and the message contains both the config file path and `left`.

### The test matrix

**Every rejection branch below gets its own test**, and each asserts the offending value appears in the message. "One test per numbered rule" is not enough — several rules carry independent branches that can each regress alone.

| # | Rejection branch | Message must contain |
|---|---|---|
| 1 | `default_tab_layout` names no layout | the bad name |
| 2 | pane `agent` names no agent | layout, pane, bad agent name |
| 3 | pane `option` names no option of that agent | layout, pane, agent, bad option name |
| 4 | `option` set with `agent` omitted | layout, pane, the option |
| 5a | layout declares no panes | the layout name |
| 5b | first pane is not `type = "agent"` | layout, the found type |
| 6 | two or more `type = "agent"` panes | layout, the count |
| 7 | duplicate pane name in a layout | layout, the name |
| 8a | `split_from` names a later pane or an unknown one | layout, pane, the target |
| 8b | root pane sets `split_from` | layout, the root pane name |
| 9a | non-root pane omits `direction` | layout, pane |
| 9b | root pane sets `direction` | layout, the root pane name |
| 10a | non-root pane omits `ratio` | layout, pane |
| 10b | root pane sets `ratio` | layout, the root pane name |
| 10c | `ratio = 0.0` | layout, pane, `0` |
| 10d | `ratio = 1.0` | layout, pane, `1` |
| 10e | `ratio = nan` | layout, pane |
| 11a | `type = "command"` without `command` | layout, pane |
| 11b | `command` set on `agent` or `shell` | layout, pane |
| 11c | pane `command` is empty or whitespace-only | layout, pane |
| 11d | agent `command` is an empty array | the agent name |
| 11e | option `command` override is an empty array | agent, option |
| 12a | duplicate layout name | the name |
| 12b | duplicate agent name | the name |
| 12c | duplicate option name within one agent | agent, the name |

Twenty-five rejection tests. Plus three positive tests: `Config::load()` with no file returns `Ok`; the real `config.example.toml` passes; the `direction = "left"` parse-stage test above.

Name them consistently — `fn <what>_is_a_named_error()` — so the count is greppable.

## Acceptance criteria

- [x] All the rules above are enforced in `Config::validate()`, and `validate()` is called by `Config::load()` before it returns.
- [x] Every **validation-stage** rejection message names the offending value: the layout, the pane, and the bad name or number. Parse-stage errors are left to `toml`'s path-plus-span reporting, with no custom `Deserialize`.
- [x] `split_from` is checked against panes seen so far, not against the whole list; no cycle-detection code exists.
- [x] `ratio` rejects `0.0`, `1.0`, negatives, values above 1, and `NaN`.
- [x] The root pane is rejected if it sets `direction`, `ratio`, or `split_from`; non-root panes are rejected if they omit `direction` or `ratio`.
- [x] Every one of the 25 rejection branches in the matrix above has its own test asserting the listed value appears in the message.
- [x] `Config::load()` with no config file returns `Ok`.
- [x] The real `config.example.toml` passes `Config::load()`.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo test config::` passes, including all 25 rejection tests and all three positive tests.
- [x] Count them: `rg -c 'fn .*_is_a_named_error' src/config.rs` reports at least 25.
- [x] Walk the matrix row by row and name the test function covering each. Report any row with no test.
- [x] Run `git status --short` and quote it. Expect `src/config.rs` plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A rule is enforced at launch time instead of load time, or `split_from` allows a forward reference | All rules present but `ratio` accepts `1.0` or `NaN`, or the root-pane geometry rules are one-directional | All rules fire at load time, `split_from` is earlier-only, `ratio` bounds are exclusive and `NaN`-safe |
| Test coverage | ×2 | Fewer than 25 rejection tests, or tests assert only `is_err()` | All 25 present but several omit the offending-value assertion | One test per matrix row, each asserting the value in the message, plus the three positive tests |
| Interface & readability | ×1 | A rule registry, visitor, or trait for a page of inline checks | Deeply nested closures that obscure the check order | One readable function, checks in file order, a `BTreeSet` for uniqueness |
| Assumptions & docs | ×1 | The earlier-only rule left unexplained, inviting a later cycle detector | Root-pane geometry rules unexplained | Comments explain why earlier-only removes cycles, why the root carries no geometry, and why the `NaN` guard is written inverted |

## Out of scope

- **Checking that a referenced command exists on `$PATH`** — `claude`, `ccr`, `yazi`, and `btop` may legitimately be absent on one machine and present on another, and a config is not invalid because a binary is not installed yet. The launch failure reports that at the right time.
- **Warning about an unreachable layout** — a layout no flag and no `default_tab_layout` names is dead weight, not an error. The user may be about to wire an action to it.
- **Validating `dashboard_workspace` against Herdr's live workspace list** — that needs a socket call, which is exactly what this validation runs before. The dashboard flow already reports a missing workspace by name.
- **Anything under `src/flows/`** — the launch flow starts reading layouts in the next bucket.
