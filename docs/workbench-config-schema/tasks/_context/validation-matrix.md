# Validation matrix

> The complete list of rejection branches `Config::validate()` must enforce. Two tasks read this file: the one that implements the rules, and the closing review that confirms none of them silently disappeared. It is the single copy — if a branch changes, it changes here.

## Why this lives at load time

Two entry points build a tab. `create_popup_tab` creates the tab first and closes it on any failure, so a mid-build error is cleaned up. The in-pane path `apply_launch_layout` has **no cleanup** — it splits panes into a tab that is already on screen. A layout that fails halfway there leaves a half-built tab the user has to dismantle by hand.

So validation is not spread across the flow. It runs once, in `Config::load()`, before the first socket call. A config that loads is a config that can be built.

## Two error stages, two contracts

- **Validation stage** — everything in the matrix below. Every message names the offending value: the layout, the pane, and the bad name or number.
- **Parse stage** — an unknown field, an unknown enum variant (`direction = "left"`), or a wrong type fails inside serde, before any `Config` exists. Those carry no layout or pane name and do not need to: `toml` reports the file path plus the exact line and column. **No custom `Deserialize` impl is written to inject names into parse errors.**

## The 25 rejection branches

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

**Every branch gets its own test**, and each asserts the listed value appears in the message. A test that only asserts `is_err()` does not prove the user can find the problem. Several rules carry independent branches that can each regress alone, which is why the matrix is written per branch rather than per numbered rule.

Name the tests consistently — `fn <what>_is_a_named_error()` — so `rg -c 'fn .*_is_a_named_error' src/config.rs` is a meaningful count. It must report at least 25.

## The three positive tests

1. `Config::load()` with no config file returns `Ok` — the built-in defaults are valid by construction.
2. The real `config.example.toml` passes `Config::load()`.
3. A config with `direction = "left"` fails at the parse stage with a message containing both the config file path and `left` — proving parse errors are actionable without custom deserialization.

## Notes that shape the implementation

- **5a is checked first, before any rule that indexes `panes[0]`.** `panes` deserializes with `#[serde(default)]`, so a `[[tab_layouts]]` carrying only a `name` is valid TOML producing an empty `Vec`. Without this ordering the root access panics instead of reporting a named error.
- **8a is "earlier", and that is the whole rule.** Panes are created top to bottom, so a pane can only split something that already exists. Enforcing earlier-only makes a forward reference, a self reference, and a cycle the same error. **There is no cycle to detect — do not build a graph walker.** Membership in the set of names seen so far is sufficient and complete.
- **10c/10d/10e come from one guard.** Written as `!(ratio > 0.0 && ratio < 1.0)`, it rejects both bounds and `NaN` for free, because every comparison against `NaN` is false. The inverted form looks like a clippy target, so it carries a comment saying why.
- **9 needs no value check.** The `Direction` enum already restricts the value to `right` or `down` at deserialization time, because Herdr's `pane.split` accepts no others. Only presence is validated.
- **11c/11d/11e exist because empty is not absent.** `Vec<String>` and `String` deserialize happily from `[]` and `""`, and an empty argv fails inside the launch flow — after the tab is on screen. That is the exact failure this whole matrix prevents.
- **Shape**: a plain nested loop over layouts and panes with a `BTreeSet` of seen names. No visitor, no rule registry, no trait. One function that reads top to bottom in the same order the user reads their file. Validate layouts in file order and panes in list order, so the first error reported is the first error in the file.
