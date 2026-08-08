# ADR-0001: Hand-rolled display_width table instead of the unicode-width crate

- Status: Accepted
- Date: 2026-07-31

## Context

Centering the agent/option menu in `src/flows/agent.rs` requires knowing each row's on-screen display width so the block lines up the way `gum` actually renders it. Menu labels can include Nerd Font Private Use Area glyphs and CJK text (branch names may be Chinese), and Rust's `chars().count()` undercounts both, so it cannot drive the alignment math.

## Considered alternatives

- Depend on the `unicode-width` crate for column-width measurement. Rejected: this code path is invoked via `project source`, which re-execs on every fzf keypress, and Cargo.toml already treats startup cost as a documented budget line; adding a dependency for a handful of code ranges was judged not worth that cost.
- Center each option label individually. Rejected: it produces a ragged left edge, so glyphs across rows don't line up into a straight column.
- Hand-write a local `display_width` match/range table calibrated against `gum`/`lipgloss`'s own measurement, and left-align the whole option block as a group, centering the block using only the widest row's width. Chosen.

## Decision

`src/flows/agent.rs` computes display width with a local, hand-written table instead of the `unicode-width` crate. The table is calibrated empirically to match `gum`/`lipgloss`'s real rendering: Nerd Font PUA glyphs count as 1 column, and CJK ranges count as 2 columns. The option block is left-aligned as a unit and then centered using the widest row's computed width, rather than centering each row independently.

## Consequences

- Menu alignment stays correct for option labels that are agent names or branch names containing Nerd Font icons or CJK characters.
- No `unicode-width` dependency was added; the startup cost of the `project source` re-exec on every fzf keypress is unaffected.
- The table only covers the code ranges actually observed against gum/lipgloss's measurement and this codebase's real inputs. A future script or PUA range outside those ranges will compute an incorrect width until someone extends the table by hand; this is an accepted maintenance cost, not full Unicode coverage.

## Evidence

- **Gum/lipgloss's own column-width table required a local match, not `chars().count()`** — measured Nerd Font PUA glyphs (e.g. `\u{f15ce}`) at 1 column and CJK at 2 columns (`中文分支` = 8 cols, `こんにちは` = 10 cols) against gum's real rendering; `unicode-width` was rejected because this script's Cargo.toml already budgets startup cost, being re-exec'd via `project source` on every fzf keypress, and per-option centering was rejected because it left glyphs unaligned into a straight column.
  Session `c1be22d7-3263-4e07-a302-5c2f9b08ebb2`, entry `474d1722-b2f6-453b-9ddf-c40d04c9d4c0`, 2026-07-31.
