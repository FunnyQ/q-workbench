# ADR-0004: New-tab menu lists layouts in config order, not default-first

- Status: Accepted
- Date: 2026-08-21

## Context

`src/flows/tab.rs`'s `ordered_layouts()` previously called `agent::resolve_layout(config, None)` to pull `default_tab_layout` out to the first row, with the rest of the layouts following config order behind it. That baked a positional assumption into the menu: the default layout was always row one regardless of where it appeared in the config file, and that assumption was encoded directly into positional test assertions, so nothing would flag a maintainer reverting to "pin the default on top" behavior later.

## Considered alternatives

- Order layouts by most-recent-use. Rejected: implicit and unpredictable — a maintainer editing the config file couldn't predict the resulting menu order just by reading it.
- Keep `default_tab_layout` pinned to the top row. Rejected: it couples two separate choices into one action — changing which layout is the default and changing which layout draws first became the same edit, so a maintainer who only wanted to change the default was forced to also reorder the menu.

## Decision

`ordered_layouts()` now returns `config.tab_layouts.clone()` directly instead of routing through `agent::resolve_layout(config, None)`. The new-tab menu lists layouts strictly in the order they are written in config; `default_tab_layout` only selects what a flagless launch opens, it no longer moves that layout's row. The blank-tab row stays pinned as the menu's last row (via `config::BLANK_LAYOUT_NAME`) regardless of config order — this change does not touch that pin.

## Consequences

- Reordering the new-tab menu is now purely a config-file edit — moving a `[[tab_layouts]]` table moves its row in the menu, with no separate setting to also update.
- Changing `default_tab_layout` no longer has the side effect of moving a row; it only changes what a flagless `agent launch` opens.
- `ordered_layouts()` no longer has a failure path now that it doesn't call `agent::resolve_layout`, so its return type narrowed from `Result<Vec<TabLayout>>` to `Vec<TabLayout>`; any caller still expecting a `Result` needed updating.
- Positional test assertions that encoded "the default row is first" needed rewriting to assert against config order instead.

## Evidence

- **Pinning the default row coupled two separate choices into one edit** — `ordered_layouts()` called `agent::resolve_layout(config, None)` to extract `default_tab_layout` into row one; changing which layout drew first required editing `default_tab_layout` even when the maintainer only wanted a different default, not a different menu order. Switching to a direct `config.tab_layouts.clone()` also removed the function's only failure path, narrowing its return type from `Result<Vec<TabLayout>>` to `Vec<TabLayout>`; the blank-tab row's last-position pin was unaffected.
  Session `3889d50b-9a7a-4184-95c0-3db129c988f2`, entry `e7dbb8ed-f6e4-48ac-9e6e-75c765381c4f`, 2026-08-21.
