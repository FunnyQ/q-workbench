# WORK-04: The layout menu flow in `src/flows/tab.rs`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: work/01, work/02, work/03
> **Status**: todo
> **Blocks**: work/05, work/07

## Goal

Add a flow that asks which tab layout to use and then runs the existing popup flow with the
chosen layout.

## Files to create / modify

- `src/flows/tab.rs` (new) — `choose_layout`, `new_with`, and `new`, plus their unit tests
- `src/flows/mod.rs` (modify) — declare the module

Nothing else. `src/flows/agent.rs` already exposes everything this flow calls; if it does
not compile against it, report that rather than editing `agent.rs` here.

## Implementation notes

### What already exists

These are available and must be used rather than reimplemented:

```rust
// src/flows/menu.rs
pub(crate) trait Menu {
    fn choose(&mut self, title: &str, subtitle: &str, options: &[String], height: u8)
        -> Result<Option<String>>;
    // filter(...) and input(...) also exist; this flow needs neither
}
pub(crate) struct GumMenu;
impl GumMenu { pub(crate) fn new(cols: u16, lines: u16) -> Self; }
pub(crate) fn strip_pad(value: &str) -> String;

// src/config.rs
impl TabLayout { pub fn menu_label(&self) -> String; }   // rendered row, unique across layouts
impl Config {
    pub fn layout(&self, name: &str) -> Option<&TabLayout>;
}
// Config fields: pub default_tab_layout: String, pub tab_layouts: Vec<TabLayout>

// src/flows/agent.rs
pub(crate) fn popup_with_layout(
    client: &dyn HerdrClient, config: &Config, layout: &TabLayout, worktree: bool) -> FlowResult;
pub(crate) fn popup_viewport() -> (u16, u16);   // terminal_size → COLUMNS/LINES → tput → 80

// src/flows/mod.rs
pub enum Outcome { Done, Cancelled, Notice { title: String, body: String } }
pub type FlowResult = anyhow::Result<Outcome>;
```

`popup_with_layout` adopts the invoking pane's cwd itself, as its first step. This flow
must not do it too.

`Config::validate()` guarantees at load that `default_tab_layout` names an existing layout,
that layout names are unique, and that no two layouts render the same `menu_label()`. Rely
on all three; do not re-check them at runtime.

### `choose_layout`

```rust
/// Ask which tab layout to use.
///
/// `Ok(None)` means the user cancelled, which is normal and quiet.
fn choose_layout<'a>(config: &'a Config, menu: &mut impl Menu) -> Result<Option<&'a TabLayout>>;
```

Behaviour:

1. **One layout — no menu.** When `config.tab_layouts.len() == 1`, return that layout
   without calling `menu`. A menu with one row asks nothing.
2. **Ordering.** Build the option list as the `default_tab_layout` entry first, then every
   other layout in `config.tab_layouts` order. Do not sort.
3. **Rows.** Each option is `layout.menu_label()`.
4. **Selection.** Call `menu.choose(...)`, then `strip_pad` the result. Both `None` from
   the menu and an empty string after stripping mean cancelled — the same two shapes the
   harness menu handles.
5. **Reverse lookup.** Find the layout whose `menu_label()` equals the stripped selection.
   Uniqueness is enforced at config load, so `.expect("validated at load")` is the right
   shape here, matching the harness menu.

Menu chrome, matching the existing menus' style:

- Title: a Nerd Font glyph, two spaces, then a short noun phrase. Use `\u{eb03}` (a layout
  glyph) followed by two spaces and `Tab Layout`. Declare it as a `const` beside the flow,
  the way `agent.rs` declares `HARNESS_TITLE`, and write the escape rather than pasting the
  glyph.
- Subtitle: `Choose a layout.`
- Height: `8`, the same as the harness menu.

### `new` and `new_with`

Split the entry point so the menu is injectable. `new` owns the two things a test cannot
supply — the config file and the real terminal — and `new_with` holds the logic:

```rust
/// Pick a tab layout, then open the agent popup for it.
pub fn new(client: &dyn HerdrClient) -> FlowResult {
    // Config first: the flow must report a broken config before it issues its own first
    // request.
    let config = Config::load().context("failed to load config")?;
    let (cols, lines) = agent::popup_viewport();
    let mut menu = GumMenu::new(cols, lines);
    new_with(client, &config, &mut menu)
}

fn new_with(client: &dyn HerdrClient, config: &Config, menu: &mut impl Menu) -> FlowResult {
    // The layout menu draws before anything touches the socket: it does not depend on the
    // project directory, and `popup_with_layout` adopts the invoking pane's cwd with a
    // `pane.get`. Cancelling here therefore issues no request at all.
    let Some(layout) = choose_layout(config, menu)? else {
        return Ok(Outcome::Cancelled);
    };
    // `tab new` never opens a worktree; the worktree action covers that case.
    agent::popup_with_layout(client, config, layout, false)
}
```

This mirrors the `choose_agent` / `choose_agent_with` seam that already exists in
`agent.rs`. Keep `new_with` private to the module — only the tests and `new` call it.

### Tests

Add a `#[cfg(test)] mod tests` at the bottom of `src/flows/tab.rs`. Use a local `FakeMenu`
implementing `Menu` with a queued script of answers and a record of the `options` it was
handed — the popup tests in `agent.rs` have the same shape to copy. Use
`Config::test_default()` and mutate `tab_layouts` / `default_tab_layout` to build cases;
build extra layouts by cloning the built-in one and changing `name`.

Cover:

- Ordering: with three layouts where the default is second in config order, the options
  passed to `choose` are `[default, first, third]`.
- Rendering: a layout with `label` and `icon` appears as its `menu_label()`, not its `name`.
- Single layout: with exactly one layout, `choose_layout` returns it and `FakeMenu` records
  zero calls.
- Cancel via `None`: `choose_layout` returns `Ok(None)`.
- Cancel via empty selection: a menu answering `""` returns `Ok(None)`.
- Reverse lookup: selecting the third row returns the third layout, and the padded form
  (`"   " + menu_label()`) resolves to the same layout.
- Cancellation costs nothing: `new_with`, given a config with two or more layouts, a
  `FakeMenu` that cancels, and a `FakeClient`, returns `Outcome::Cancelled` with
  `client.calls` empty. Use two layouts, not one — a single-layout config skips the menu
  and proceeds into tab creation, which is a different path.
- The chosen layout is the one built. Details below — this one needs care.

### The "chosen layout is built" test

`new_with`'s `FakeMenu` controls the **layout** menu only. `popup_with_layout` goes on to
call `choose_agent`, which builds its own real `GumMenu` internally. So the test must use
layouts that make `choose_agent` run **no menu at all**. A layout does that when its root
pane pins the agent and the layout pins the tab label:

```rust
// A layout whose every choice is pinned: choose_agent runs no menu, so no `gum` subprocess.
fn pinned_layout(name: &str, tab_label: &str) -> TabLayout { /* panes[0]: type = agent,
    agent: Some("codex".into()), option: None, plus tab_label: Some(tab_label.into()) */ }
```

`codex` in the built-in agents has no options, so the model menu is skipped too, and
`worktree: false` skips the branch menu.

**Pin the environment.** `adopt_invoking_pane_cwd` issues a `pane.get` when
`HERDR_ACTIVE_PANE_ID` is set and `HERDR_PLUGIN_CONTEXT_JSON` is absent, which would insert
an unqueued call and desynchronise the expected sequence. Tests that run inside a Herdr
pane inherit both variables, so the test must control them rather than assume them. The
repository already has the mechanism: take `crate::state::env_lock()` for the duration,
save the current values of `HERDR_ACTIVE_PANE_ID` and `HERDR_PLUGIN_CONTEXT_JSON`, remove
both, and restore them before asserting. `a_broken_config_stops_the_popup_before_the_first_socket_call`
in `agent.rs` is the pattern to copy, including the guard and the save/restore loop. Apply
the same treatment to the cancellation test, which asserts on an empty call list.

Give the two layouts different side panes so their call sequences differ. Queue the
`FakeClient` the same responses the existing popup tests do — `tab.create` returning
`{"root_pane": {"pane_id": …}, "tab": {"tab_id": …}}`, then one `pane.split` response per
non-root pane. Then assert the recorded calls carry the **selected** layout's `tab_label`
in `tab.create` / `tab.rename` and one `pane.split` per non-root pane of that layout. The
claim under test is that selecting the second layout does not build the first.

An unpinned layout in this test would shell out to `gum`. If the test appears to hang or
draws a menu, a layout is not fully pinned.

`new` itself runs `Config::load()` and the real `GumMenu`, so every test drives
`choose_layout` or `new_with`. There is no test of `new`.

## Acceptance criteria

- [ ] `src/flows/tab.rs` exists, is declared in `src/flows/mod.rs`, and exposes
      `pub fn new(client: &dyn HerdrClient) -> FlowResult`.
- [ ] `new_with(client, &Config, &mut impl Menu)` holds the logic and is driven by the
      tests; `new` only supplies the loaded config and the real `GumMenu`.
- [ ] The menu lists the `default_tab_layout` first and the rest in `config.tab_layouts`
      order, each row rendered by `TabLayout::menu_label()`.
- [ ] A config with exactly one layout draws no menu.
- [ ] Cancelling the layout menu returns `Outcome::Cancelled` and the flow issues no
      socket call, proven against a `FakeClient` with two or more layouts configured.
- [ ] Selecting a non-default layout builds that layout's tab, proven by the recorded
      `FakeClient` call sequence.
- [ ] `new_with` passes `worktree: false` and never runs the worktree menu.
- [ ] The flow calls the existing `popup_with_layout`; it does not re-implement tab
      creation, and does not adopt the invoking pane's cwd a second time.
- [ ] `src/flows/agent.rs` is not modified by this task.
- [ ] Every test that asserts on a `FakeClient` call list holds `state::env_lock()` and
      removes `HERDR_ACTIVE_PANE_ID` and `HERDR_PLUGIN_CONTEXT_JSON`, restoring both
      afterwards, so it passes whether or not it runs inside a Herdr pane.
- [ ] No `expect`/`unwrap` on anything config validation does not already guarantee.

## Verification

- [ ] `cargo test` passes, including the new `src/flows/tab.rs` tests.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `git status --short -- src/flows/tab.rs src/flows/mod.rs` shows both paths dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Wrong order, menu drawn for a single layout, or tab creation duplicated | Happy path works but a cancel shape or the padded selection is mishandled | Ordering, single-layout skip, both cancel shapes, and the reverse lookup all correct |
| Test coverage | ×2 | No tests | Ordering only | Ordering, rendering, single layout, both cancel shapes, padded lookup, zero socket calls on cancel, and the chosen layout's tab actually built |
| Interface & readability | ×1 | Menu construction inlined into `new`, leaving the flow untestable | Testable but signatures leak `GumMenu` | `choose_layout` and `new_with` take `&mut impl Menu` and borrow the config; `new` is a thin caller |
| Assumptions & docs | ×1 | No note on why the menu precedes cwd adoption | Comment present but vague | The ordering reason and the "validated at load" assumption are both stated |

## Out of scope

- Wiring the CLI subcommand or the plugin manifest. A later task owns both.
- A `--layout` flag or any way to skip the menu from the command line.
- Worktree support. `tab new` never opens a worktree; the existing worktree action covers
  that case.
- Remembering the last chosen layout. The stored last-agent record is keyed per pane and a
  popup has no pane of its own.
- Touching `src/flows/layout.rs`, which is the unrelated pane split-ratio flow.
