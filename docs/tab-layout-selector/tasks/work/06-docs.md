# WORK-06: Document the action and the two new layout keys

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: work/02, work/05
> **Blocks**: work/07
> **Status**: done

## Goal

Describe the `new-tab` action in `README.md` and the new `label` / `icon` layout keys in
both `README.md` and `config.example.toml`, so a reader can use them without reading the
Rust.

## Files to create / modify

- `README.md` (modify) — actions table, action count, keybinding table, configuration
  section
- `config.example.toml` (modify) — the layout keys block

## Implementation notes

### What shipped

- A Herdr action `new-tab`, titled `New tab`, opening a popup that lists the configured tab
  layouts and then runs the normal agent popup for the chosen one. It never opens a
  worktree.
- Two optional keys on a `[[tab_layouts]]` entry:
  - `label` — the layout menu row. Omit it and the row shows `name`.
  - `icon` — drawn before the label, separated by two spaces: `"${icon}  ${label}"`.
- Both keys reject an empty string at config load, and two layouts may not render the same
  menu row.
- The menu lists `default_tab_layout` first, then the rest in config order, and is skipped
  entirely when only one layout is configured.

### `README.md`

1. **"What it does"** opens with `The plugin exposes eight actions.` That count is now
   nine. Add a row to the actions table, placed after `new-assistant` so the agent-launching
   actions stay together:

   | Action | What happens |
   | --- | --- |
   | `new-tab` | Pick a tab layout, then the usual harness → model → usage menus, and open a tab from that layout |

2. **"Bind it"** — add a row to the suggested-bindings table. Pick a key that does not
   collide with `alt+c`, `alt+shift+c`, `alt+p`, `alt+s`, `alt+r`, `prefix+d`, or
   `prefix+e`. `alt+t` is the natural free choice.

   The paragraph under that table explains why the two agent actions are paired on
   `alt+c` / `alt+shift+c` — "the worktree-vs-normal choice *is* the keybinding, which is
   why neither action prompts". Keep that claim true: say in one sentence that `new-tab`
   asks which layout instead, and still does not ask about worktrees.

3. **"Configuration"** — the prose says `Omitting a layout choice makes the launcher ask
   for it.` Extend that area with the two new keys and their fallback, and mention that the
   layout menu row must be unique across layouts. Do not restate the whole schema; the
   section already defers to `config.example.toml`.

   The settings table lists top-level settings only, so it needs no new row. The
   `default_tab_layout` row reads `Layout used when a launch does not pass --layout`, which
   is still true — but the `new-tab` menu now also hoists that layout to the top, so it is
   worth a clause.

### `config.example.toml`

The tab-layouts section documents the layout keys in a comment block:

```
# Layout keys:
#   name       Stable id. A workspace and default_tab_layout name it.
#   tab_label  Tab title, and with it the title of the agent pane. Omit to ask: …
```

Add `label` and `icon` to that list, in the field order they now have on the struct
(`name`, `label`, `icon`, `tab_label`). Follow the file's existing wording style — the pane
key list already describes an `icon` the same way ("Drawn before the label, separated by two
spaces"), so mirror it rather than inventing new phrasing.

The block above that list explains how a layout is selected: "A layout is selected with
`--layout <name>` … Without the flag the launcher uses `default_tab_layout`." Add the third
route: the `new-tab` action asks, listing every layout with `default_tab_layout` first, and
skips the menu when only one layout is defined.

If the example file gains a `label` or `icon` value on a layout, the parse test
`the_example_config_parses_with_no_unknown_fields` covers it; keep any added value non-empty,
because an empty string is a load error.

### Style

`README.md` and `config.example.toml` are user-facing English. Write one instruction per
sentence, put the condition before the instruction, use one term per thing, and copy
identifiers exactly.

## Acceptance criteria

- [x] The README action count matches the number of rows in the actions table.
- [x] The actions table has a `new-tab` row describing the layout menu.
- [x] The suggested-keybinding table has a `new-tab` row with a key that collides with no
      existing row.
- [x] The worktree paragraph stays accurate for the new action.
- [x] The README configuration section documents `label` and `icon`, their fallback to
      `name`, and the uniqueness requirement.
- [x] `config.example.toml` lists `label` and `icon` in its layout-keys comment block.
- [x] `config.example.toml` describes the third selection route: the `new-tab` menu,
      default first, skipped when only one layout exists.
- [x] No documented behaviour contradicts the shipped flow: no worktree step, no
      `--layout` flag on the new action, cancelling is silent.

## Verification

- [x] `cargo test` passes — `the_example_config_parses_with_no_unknown_fields` proves the
      example file still parses.
- [x] `grep -c '^\[\[actions\]\]' herdr-plugin.toml` returns 9, and the README sentence
      that counts the actions says nine, and the README actions table has nine rows.
- [x] `git status --short -- README.md config.example.toml` shows both paths dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Documents behaviour the code does not have, or leaves the action count wrong | Accurate but incomplete — one of the two files or one of the two keys missing | Every documented claim matches the shipped flow, both files updated |
| Test coverage | ×2 | The example file no longer parses | Parses, but an added value is untested by the existing parse test | `cargo test` green, example file exercised by the parse test |
| Interface & readability | ×1 | New prose diverges from the file's voice | Readable but wordy or restates the schema | Matches the existing style, one instruction per sentence, no duplication of the schema |
| Assumptions & docs | ×1 | Fallback and uniqueness rules unstated | Stated vaguely | Fallback chain, empty-string rejection, and uniqueness all stated plainly |

## Out of scope

- CHANGELOG and version bump — a release step owns both.
- Rewriting sections the change does not touch.
- Adding a keybinding to the plugin. The table is a suggestion; the plugin ships none.
- Translating any documentation. These files stay English.
