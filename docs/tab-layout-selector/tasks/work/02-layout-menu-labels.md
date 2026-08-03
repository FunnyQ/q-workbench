# WORK-02: Give tab layouts a menu label

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: none — foundation task
> **Blocks**: work/04, work/06, work/07
> **Status**: todo

## Goal

Let a `[[tab_layouts]]` entry carry an optional `label` and `icon`, render them into one
menu row through the existing helper, and reject at config load the two ways that row can
go wrong.

## Files to create / modify

- `src/config.rs` (modify) — two new fields on `TabLayout`, a `menu_label()` method, and
  two new validations plus their tests

## Implementation notes

### The fields

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabLayout {
    pub name: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub tab_label: Option<String>,
    #[serde(default)]
    pub panes: Vec<LayoutPane>,
}
```

Both are optional and both default to `None`. `name` stays the stable id: `--layout`,
`default_tab_layout`, and every lookup keep matching on `name`.

`TabLayout` is constructed in `default_tab_layouts()` and in tests, so every literal needs
the two new fields. The built-in `agentic-coding` layout gets `label: None, icon: None` —
it keeps rendering as its bare name.

### The rendered row

Mirror `Agent::menu_label` exactly, including the reason in the doc comment:

```rust
impl TabLayout {
    /// The layout menu row for this layout. The reverse lookup in the layout menu matches
    /// on this exact string, so every site that renders a layout must go through here.
    pub fn menu_label(&self) -> String {
        render_label(
            self.icon.as_deref(),
            self.label.as_deref().unwrap_or(&self.name),
        )
    }
}
```

`render_label(icon, label)` already joins them with exactly two spaces and returns the
label alone when there is no icon.

### Validation

Both checks belong in `Config::validate()`, in the loop that already walks
`self.tab_layouts` collecting `layout_names`. Every message names the offending layout, and
uses `bail!` like its neighbours.

1. **Empty strings.** A written-but-empty `label` or `icon` renders a blank or
   glyph-only row, which is unusable and almost certainly a typo. Omitting the key is the
   supported way to get the fallback.

   ```rust
   if layout.label.as_deref().is_some_and(str::is_empty) {
       bail!("layout '{}': label is empty; omit the key to fall back to the name", layout.name);
   }
   if layout.icon.as_deref().is_some_and(str::is_empty) {
       bail!("layout '{}': icon is empty; omit the key to render the label alone", layout.name);
   }
   ```

2. **Duplicate rendered rows.** The layout menu returns the rendered row and maps it back
   to a layout by that string, so two layouts rendering the same row would both be listed
   while only the first could ever be selected. Collect rendered labels in a
   `BTreeMap<String, &str>` keyed by label, valued by layout name, exactly as the agent
   loop does:

   ```rust
   bail!(
       "layouts '{}' and '{}' render the same menu label: {}",
       other, layout.name, label
   );
   ```

   Two layouts can collide even with distinct `name`s — `name = "a"` with `label = "Work"`
   against `name = "Work"` with no label.

### Tests to add

Add to the existing `#[cfg(test)] mod tests` in `src/config.rs`:

- `menu_label` falls back to `name` when both keys are omitted.
- `menu_label` uses `label` when set, and joins `icon` and `label` with two spaces.
- A TOML config with `label` and `icon` on a layout parses (the loader path, not just the
  struct).
- An empty `label` fails to load, and the error names the layout.
- An empty `icon` fails to load, and the error names the layout.
- Two layouts rendering the same menu label fail to load, and the error names both.
- A layout whose `label` equals another layout's `name` collides too.

Config tests that need a file on disk write TOML into a temporary directory and point
`Q_WORKBENCH_LOCAL_CONFIG` at it; follow the existing tests in the same module. Tests that
only need a value can start from `Config::test_default()` and mutate it.

Two existing tests pin the config surface and may need updating: the one asserting
`config.tab_layouts == default_tab_layouts()`, and
`the_example_config_parses_with_no_unknown_fields`, which parses `config.example.toml`.
Update them only as far as the new fields require.

## Acceptance criteria

- [ ] `TabLayout` has `pub label: Option<String>` and `pub icon: Option<String>`, both
      optional in TOML.
- [ ] `TabLayout::menu_label()` returns `icon + two spaces + label`, falling back to
      `label` alone without an icon and to `name` without a label.
- [ ] An empty `label` or an empty `icon` is a load-time error naming the layout.
- [ ] Two layouts whose `menu_label()` is equal are a load-time error naming both.
- [ ] `name` remains the id used by `--layout`, `default_tab_layout`, and `Config::layout`.
- [ ] The built-in `agentic-coding` layout is unchanged on screen: no label, no icon.

## Verification

- [ ] `cargo test` passes, including the new config tests.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `git status --short -- src/config.rs` shows the file dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Fields required rather than optional, or `name` no longer the id | Fields work but a validation is missing or fires on the wrong input | Both fields optional, fallback chain correct, both validations fire and name the layout |
| Test coverage | ×2 | No new tests | Fallback covered, failure paths not | Fallback, both empty-string errors, and both duplicate-label shapes covered |
| Interface & readability | ×1 | Bespoke rendering instead of `render_label` | Works but diverges from the agent pattern | `menu_label()` mirrors `Agent::menu_label` and reuses `render_label` |
| Assumptions & docs | ×1 | No note on why duplicates are rejected | Comment present but vague | The reverse-lookup reason is stated where the check lives |

## Out of scope

- The layout menu itself. This task only makes a layout renderable and validated.
- Documenting the keys in `README.md` or `config.example.toml`. A later documentation task
  owns both files.
- Any change to `LayoutPane`, which already has its own `label` and `icon`.
- Changing how `default_tab_layout` is resolved.
