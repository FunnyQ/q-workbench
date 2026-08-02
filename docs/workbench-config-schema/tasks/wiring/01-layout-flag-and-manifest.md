# WIRING-01: Layout flag and manifest entries

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: launch/01, launch/03
> **Blocks**: wiring/02, wiring/03
> **Status**: done

## Goal

A user reaches any named tab layout with `--layout <name>` from the CLI or from a Herdr action, and the flow carries that layout through every entry point including reinjection.

## Files to create / modify

- `src/flows/agent.rs` (modify) — `LaunchOptions` and `InjectOptions` gain a `layout` field; `popup`, `launch`, and `inject` resolve it.
- `src/main.rs` (modify) — `--layout <NAME>` on the `agent popup`, `agent launch`, and `agent inject` clap definitions, threaded into the option structs.
- `herdr-plugin.toml` (modify) — one new action and one new popup pane for the pinned layout.
- `src/flows/picker.rs` (modify) — the `agent::inject` call site gains `layout: None`.

## Implementation notes

### What already exists when this task starts

The config loader exposes resolved layouts and a default pointer. Inline signatures:

```rust
pub struct TabLayout {
    pub name: String,
    pub tab_label: Option<String>,
    pub panes: Vec<LayoutPane>,
}

// on Config
pub default_tab_layout: String,
pub fn layout(&self, name: &str) -> Option<&TabLayout>
```

`Config::load()` has already validated that `default_tab_layout` resolves to a real layout, and that every layout is internally consistent. **The only lookup that can fail at runtime is a name the user typed after `--layout`.** Do not re-validate the default; do not duplicate layout validation here.

### Option structs

Both structs live in `src/flows/agent.rs`, currently at lines 36-51. Add one field to each:

```rust
pub struct LaunchOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
    pub no_layout: bool,
    pub restart: bool,
    pub layout: Option<String>,   // new
}

pub struct InjectOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
    pub layout: Option<String>,   // new
}
```

`None` means "use `default_tab_layout`". It does not mean "no layout" — that is what the separate `no_layout` flag already means, and the two must not be conflated.

### Resolution helper

Resolve once, early, in each entry point, before any socket call:

```rust
fn resolve_layout<'a>(config: &'a Config, requested: Option<&str>) -> Result<&'a TabLayout> {
    let name = requested.unwrap_or(&config.default_tab_layout);
    config
        .layout(name)
        .with_context(|| format!("unknown tab layout: {name}"))
}
```

The error must name the offending value — that is the house style throughout this codebase. Because `Config::load()` validated the default, a failure here always means the user typed a bad `--layout`.

Call it at the top of **all three** entry points, before any socket call:

- `popup()` (currently line 244, right after `Config::load()`).
- `launch()` (currently line 54, **before** `pane_get`). Resolving before `pane_get` matters: an unknown name must produce zero socket calls, not a rename followed by a failure.
- `inject()` (currently line 113, before the `pane_rename` at line 136). `inject()` takes no `Config` today, so it must load one. Without this, `agent inject --layout typo` renames the pane and types a command into it, and the bad name is only reported later by the nested `agent launch` — after two socket calls and a visibly mangled pane. `inject()` also needs the resolved layout anyway, to read the root pane's `label`.

Each of the three gets its own test asserting an unknown `--layout` produces an error naming the value and **zero recorded calls** on the `FakeClient`.

### CLI flags

The agent command tree is in `src/main.rs` around lines 265-307. Add `--layout <NAME>` to `agent popup`, `agent launch`, and `agent inject`. It is optional everywhere, takes one value, and maps straight into the new option field. Follow the existing clap style in that file — the neighbouring `--usage`, `--tab`, and `--worktree` flags are the pattern to copy.

### Reinjection carries the flag

`inject()` (currently lines 113-146) builds an argv for a nested `workbench agent launch` invocation and types it into the pane's shell. It must forward the layout:

```rust
if let Some(layout) = &options.layout {
    argv.extend(["--layout".to_owned(), layout.clone()]);
}
```

Build the final string with `build_command()` from `src/shell.rs`, which quotes every argument separately. This is not cosmetic: the text crosses a shell boundary, and a layout name containing a space, a quote, or a shell metacharacter must not change argv. Do not join arguments by hand.

### The open assumption — flag this in your handoff

`inject()` currently renames the pane at line 136 using a fixed constant:

```rust
const AGENT_LABEL: &str = "\u{f169f}  agent";
```

This rename happens **before any menu has run**, so no chosen label exists yet. Implement this assumption:

- When the resolved layout's root pane sets a `label`, render it with the icon convention (`"{icon}  {label}"`, exactly two spaces; label alone when there is no icon) and use that.
- Otherwise keep today's `AGENT_LABEL` constant unchanged.

Write a short comment explaining *why* the constant is still the fallback: at inject time the usage menu has not run, so there is nothing better to name the pane. If this reads wrong to you once you see the code, say so in your handoff rather than inventing a third behaviour.

Never type or paste a Nerd Font glyph into a Rust source file — write `\u{f169f}` escapes. The `Edit` tool silently drops plane-15 codepoints.

### Manifest entries

Add one action and one popup pane to `herdr-plugin.toml`, mirroring the existing `new-agent` / `agent` pair:

```toml
[[actions]]
id = "new-assistant"
title = "New personal assistant"
contexts = ["workspace"]
command = ["herdr", "plugin", "pane", "open", "--plugin", "q.workbench", "--entrypoint", "assistant", "--placement", "popup", "--width", "60%", "--height", "70%"]

[[panes]]
id = "assistant"
title = "\U000F169F  personal assistant"
placement = "popup"
command = ["./bin/workbench", "agent", "popup", "--layout", "personal-assistant"]
```

**On that `title`.** The surrounding entries in this file carry a literal Nerd Font glyph (U+F169F, `nf-md-robot`). Two spaces separate it from the text. You have two safe ways to write it and one unsafe one:

- **Safest**: the TOML basic-string escape shown above, `\U000F169F`. TOML decodes `\U` plus eight hex digits, so this is byte-identical to the literal glyph and cannot be corrupted by any tool.
- **Consistent with neighbours**: the literal glyph, written with the repo's `unicode-edit` skill.
- **Never**: retyping or pasting the glyph through `Edit`, or writing the file with a bash heredoc. `Edit` silently drops plane-15 codepoints, and a heredoc drops the fifth hex digit — U+F169F becomes U+F169 followed by a literal `f`, and TOML parses the result without complaint.

Note the `[[panes]]` entry's `id` is what the action's `--entrypoint` names. They must match.

### The project picker passes `layout: None` on purpose

`src/flows/picker.rs` around lines 265-274 calls `agent::inject` with `usage: Some(PROJECT_MAIN_LABEL)`. Add `layout: None` and leave it there. A project workspace is meant to land on `default_tab_layout` — it is not tied to any named layout. This is intentional; do not "fix" it into a lookup.

## Acceptance criteria

- [x] `LaunchOptions` and `InjectOptions` each carry `layout: Option<String>`, and every construction site compiles with it set.
- [x] `agent popup`, `agent launch`, and `agent inject` each accept an optional `--layout <NAME>`.
- [x] An unknown `--layout` value produces an error whose message contains that value.
- [x] Omitting `--layout` resolves `default_tab_layout`.
- [x] `inject()` forwards `--layout` into the argv it builds, quoted through `build_command()`.
- [x] `inject()` names the pane from the layout's root-pane label when one is set, else from the existing `AGENT_LABEL` constant, with a comment explaining the fallback.
- [x] `herdr-plugin.toml` gains a `new-assistant` action and an `assistant` popup pane whose `--entrypoint` and `id` match, and whose title glyph is byte-identical to U+F169F followed by two spaces.
- [x] The project picker's inject call passes `layout: None`.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] A test asserts that `launch` with an unknown layout name returns an error naming it and that the `FakeClient` recorded **zero** calls.
- [x] A test asserts that omitting the flag resolves the layout named by `default_tab_layout`.
- [x] A test round-trips an inject argv through `build_command()` with a layout name containing a space, then splits it back with `zsh -c 'set -- ...; printf "%s\0" "$@"'` and asserts the layout name arrives as exactly one argument. The existing round-trip test in `src/flows/dashboard.rs` is the shape to copy.
- [x] Run `python3 -c "print([hex(ord(c)) for c in open('herdr-plugin.toml').read() if ord(c) > 0xFFFF])"` and confirm every result is `0xf169f` or another intended plane-15 glyph — no `0xf169`.
- [x] Run `git status --short` and quote it. Expect `src/flows/agent.rs`, `src/main.rs`, `src/flows/picker.rs`, `herdr-plugin.toml`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Layout resolution happens after a socket call, or an unknown name silently falls back to the default | Resolution is correct but the flag does not survive reinjection, or the manifest glyph is corrupted | Resolves before any socket call, names the bad value, survives `build_command()` quoting, glyph byte-identical |
| Test coverage | ×2 | No test for the unknown-name path | Happy path only; no assertion that zero socket calls were made | Unknown name, default fallback, and a quoted round-trip with a space in the name all covered |
| Interface & readability | ×1 | `layout` and `no_layout` conflated, or resolution duplicated in each entry point | One shared helper but unclear ownership of the default | One `resolve_layout` helper, `None` clearly documented as "use the default", flags read like their neighbours |
| Assumptions & docs | ×1 | The inject-time label choice is silent | Implemented but unexplained | The `AGENT_LABEL` fallback carries a why-comment and is flagged in the handoff |

## Out of scope

- Adding a layout picker menu — Deferred. The chosen mechanism is one flag plus one manifest action per layout; a menu was considered and rejected because it costs an extra keystroke on every launch.
- Making the project picker choose a layout — Deferred. A project workspace intentionally lands on `default_tab_layout`.
- Any change to `src/flows/dashboard.rs` — Deferred. `dashboard_workspace` keeps its current meaning and the dashboard does not use layouts.
