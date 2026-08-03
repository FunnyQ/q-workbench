# WORK-01: Extract the gum menu primitives into `src/flows/menu.rs`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: none — foundation task
> **Blocks**: work/03, work/04, work/07
> **Status**: done

## Goal

Move the `gum` menu primitives out of `src/flows/agent.rs` into a new `src/flows/menu.rs`
so a second flow can draw the same menus, with no change to what any menu renders.

## Files to create / modify

- `src/flows/menu.rs` (new) — the `Menu` trait, `InputIndent`, `GumMenu`, `gum_output`,
  `gum_with_input`, `strip_pad`, `display_width`, the two filter-height constants, and the
  unit tests that cover them
- `src/flows/mod.rs` (modify) — declare the new module
- `src/flows/agent.rs` (modify) — `use` the moved items instead of defining them

## Implementation notes

This is a **pure move**. Do not rename anything, do not change a signature, do not reword a
doc comment, do not touch a rendered string. The only permitted edits are module
declarations, `use` lines, and visibility keywords.

### What moves

From `src/flows/agent.rs`:

- `trait Menu` and its three methods (`choose`, `filter`, `input`)
- `enum InputIndent { Centered, None }`
- `struct GumMenu` with `new`, `content_width`, `content_margin`, `block_margin`,
  `vertical_padding`, `render_banner`, `padded`, and `impl Menu for GumMenu`
- `fn gum_output`, `fn gum_with_input`
- `fn strip_pad`, `fn display_width`
- `const FILTER_HEIGHT_ARG: &str = "12"` and `const FILTER_HEIGHT: u16 = 12`

Carry every doc comment across verbatim. They record why `gum` needs stderr inherited, why
the banner is printed line by line instead of nested in a second `gum style`, why the pad is
stripped but the glyph kept, and how `display_width` was measured. Those reasons are not
recoverable from the code.

### Visibility

`agent.rs` and a later flow module both need these, so make them `pub(crate)`, not `pub`:

```rust
// src/flows/menu.rs
pub(crate) trait Menu { … }
pub(crate) enum InputIndent { Centered, None }
pub(crate) struct GumMenu { … }
impl GumMenu { pub(crate) fn new(cols: u16, lines: u16) -> Self; … }
pub(crate) fn gum_output<I, S>(args: I) -> Result<Option<String>>
where I: IntoIterator<Item = S>, S: AsRef<OsStr>;
pub(crate) fn gum_with_input(args: &[&str], input: &str) -> Result<Option<String>>;
pub(crate) fn strip_pad(value: &str) -> String;
pub(crate) fn display_width(value: &str) -> u16;
```

`GumMenu`'s private helpers (`content_width`, `content_margin`, `block_margin`,
`vertical_padding`, `render_banner`, `padded`) stay private to `menu.rs` unless a test
outside the module needs them; the `Menu` trait methods are reachable through the trait.

`src/flows/mod.rs` declares the module alongside the others. `menu` is crate-internal, so
prefer `pub(crate) mod menu;` and only widen it if the compiler demands it.

In `agent.rs`, replace the removed definitions with one import:

```rust
use crate::flows::menu::{display_width, strip_pad, GumMenu, InputIndent, Menu};
```

Some of `agent.rs`'s current `use` lines exist only for the moved code — `std::ffi::OsStr`,
parts of `std::io`, `std::process::Stdio`. Remove what `agent.rs` no longer uses and add
what `menu.rs` now needs. `cargo clippy -- -D warnings` catches leftovers.

### Tests that move with the code

The `mod popup` test block in `agent.rs` contains tests that exercise the menu primitives
directly rather than the agent flow. Move the ones whose subject is a menu primitive —
the `display_width` assertions and the `padded`/centering assertions — into a
`#[cfg(test)] mod tests` at the bottom of `menu.rs`. Keep every test that drives
`choose_agent` or the popup in `agent.rs`, including its `FakeMenu`, which now implements
the imported trait.

Two constants those tests reference live in `agent.rs` behind `#[cfg(test)]`
(`TEST_CLAUDE_LABEL`, and `USAGE_WRITE` which is not test-gated). A moved test that needs
one of those strings should inline the literal in `menu.rs` rather than making `agent.rs`
export it — `menu.rs` must not depend on `agent.rs`.

### Direction of dependency

`agent.rs` depends on `menu.rs`. `menu.rs` must not `use` anything from `agent.rs`. If a
compile error suggests otherwise, the wrong item is being moved.

## Acceptance criteria

- [x] `src/flows/menu.rs` exists and defines `Menu`, `InputIndent`, `GumMenu`,
      `gum_output`, `gum_with_input`, `strip_pad`, `display_width`, `FILTER_HEIGHT_ARG`,
      and `FILTER_HEIGHT`.
- [x] `src/flows/agent.rs` no longer defines any of them and imports them from
      `crate::flows::menu`.
- [x] `src/flows/menu.rs` contains no `use` of `crate::flows::agent`.
- [x] Every doc comment on the moved items is present in `menu.rs`, unchanged.
- [x] No rendered string, flag, height, or width constant changed value.
- [x] The menu-primitive tests named above are moved to `menu.rs` unchanged apart from
      imports and any inlined literal; every other test stays in `agent.rs`, unchanged
      apart from imports.
- [x] No test was deleted, renamed, or had an assertion weakened.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `git diff -- src/flows/agent.rs | grep '^[-+]' | grep -v '^[-+][-+]'` shows only
      removals of the moved code plus `use`-line changes — no edited logic lines.
- [x] `git status --short -- src/flows/menu.rs src/flows/mod.rs src/flows/agent.rs` shows all three paths dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A signature, string, or constant changed; `menu.rs` depends on `agent.rs` | Compiles, but a doc comment was dropped or reworded | Pure move: same signatures, same strings, same comments, dependency points one way |
| Test coverage | ×2 | Tests deleted or weakened to compile | Menu tests left in `agent.rs` wholesale | Menu-primitive tests moved to `menu.rs`, flow tests stayed, all green |
| Interface & readability | ×1 | Everything made `pub` | Mixed visibility with no reason | `pub(crate)` throughout, private helpers stay private |
| Assumptions & docs | ×1 | The stderr / banner / pad reasons lost | Comments present but shuffled | Every "why" comment intact and next to the code it explains |

## Out of scope

- Adding any new menu type or method — the trait keeps exactly three methods.
- Making `menu.rs` public outside the crate.
- Touching `src/flows/layout.rs`. Despite the name it is the pane split-ratio flow behind
  `pane even` and has nothing to do with menus or tab layouts.
- Refactoring `choose_agent` itself. Only its imports change here.
