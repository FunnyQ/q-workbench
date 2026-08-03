# Tab layout selector

Master spec. Task files under `tasks/` are self-contained; this file is the source of
truth for decisions and scope.

## Context

`herdr-plugin.toml` hard-codes one tab layout per action. `new-agent` and
`new-worktree-agent` fall through to `default_tab_layout`, and `new-assistant` pins
`--layout personal-assistant`. Adding a `[[tab_layouts]]` entry to `config.toml` therefore
reaches nobody until someone also adds a matching action to the plugin manifest and
rebuilds the binary.

This plan adds one action that asks. A new `tab new` subcommand runs a layout menu first,
then hands the chosen layout to the existing popup flow, so every configured layout
becomes reachable from a single keybinding.

## Goal

Ship a `New tab` Herdr action that presents a `gum` menu of the configured tab layouts and
then runs the existing agent popup flow with the chosen layout.

## Users

Q, driving Herdr through the `q.workbench` plugin. The change is user-visible as one new
action in Herdr's action list and two new optional keys in `config.toml`.

## Decisions

| Decision | Choice | Why |
| --- | --- | --- |
| CLI surface | New `tab new` subcommand (`Command::Tab { command: TabCommand }`) | A layout selector is not an agent-popup variant; a flag on `agent popup` would overload a command that already carries five |
| Menu labels | New optional `label` + `icon` on `[[tab_layouts]]`, rendered by the existing `render_label`, falling back to `name` | Matches how `[[agents]]` renders; `name` is a stable id and reads poorly in a menu |
| worktree | Not supported by `tab new` | The worktree-vs-normal choice is the keybinding today, and README says so explicitly; adding a worktree step would duplicate `new-worktree-agent` |
| Menu order | `default_tab_layout` first, everything else in config order | The common choice is one keypress away; the rest of the order stays under Q's control |
| One layout | Skip the menu, use that layout | A menu with one row asks nothing |
| Cancel | `Outcome::Cancelled` — silent, no notification | Matches every existing menu |
| Menu code | `Menu`, `InputIndent`, `GumMenu`, `gum_output`, `gum_with_input`, `display_width`, `strip_pad` move to a new `src/flows/menu.rs` | Two flows now draw menus; `tab.rs` depending on `agent.rs` internals points the wrong way |
| Field validation | Empty `label`/`icon` is a load-time error naming the layout; duplicate rendered menu labels are a load-time error | Mirrors the existing agent-label check — the menu maps a rendered row back to a layout, so two identical rows would make one unselectable |
| Delivery | Rust + tests + manifest + README + `config.example.toml` + rebuilt `bin/workbench` | A linked checkout runs the committed artifact, so an unrebuilt binary means the action silently does nothing |

## Non-goals

- No worktree step in `tab new`. Worktree launches keep using `new-worktree-agent`.
- No `--layout` flag on `tab new`. The menu is the point of the command.
- No "use last layout" memory. `state::LastAgentRecord` already stores a layout name, but
  it is keyed per pane and a popup has no pane of its own.
- No change to `new-agent`, `new-worktree-agent`, `new-assistant`, the project picker, or
  the restart path.
- No keybinding shipped by the plugin. It ships none today; README documents suggestions.
- No CHANGELOG entry and no version bump. `chronicle:release` owns both.
- No second platform artifact. The committed binary stays macOS arm64.

## Architecture

### Flow

```
tab new
  └─ Config::load()                     config errors before the flow's first socket call
  └─ terminal viewport                  agent::popup_viewport(), same source as the popup
  └─ tab::choose_layout(config, menu)   skipped when exactly one layout is configured
  └─ agent::popup_with_layout(client, &config, layout, worktree: false)
       └─ adopt_invoking_pane_cwd()     popup cwd is the plugin checkout, not the project
       └─ choose_agent() → realise_worktree() → create_popup_tab()
```

`agent::popup` keeps its signature and behaviour. The body after config load and layout
resolution is extracted into `popup_with_layout(client, &Config, &TabLayout, worktree: bool)`,
so both entry points share it — including the cwd adoption, which stays inside the shared
half so neither caller can do it twice.

**The ordering invariant is about the flow's own socket calls.** `main` builds the socket
client and runs the protocol guard (`ping`) before dispatching any notifying command; that
is the existing contract for every action and this plan does not change it. What must hold
is narrower: within a flow, the config is loaded and validated before that flow issues its
first request, so a broken config never leaves a half-built tab on screen.

### Files

| File | Change |
| --- | --- |
| `src/flows/menu.rs` | New. `Menu` trait, `InputIndent`, `GumMenu`, `gum_output`, `gum_with_input`, `display_width`, `strip_pad`, plus their moved unit tests |
| `src/flows/agent.rs` | Uses `menu::*`; `popup` split into `popup` + `popup_with_layout(client, &Config, &TabLayout, worktree: bool)`; `popup_viewport` widened to `pub(crate)` |
| `src/flows/tab.rs` | New. `choose_layout` (default-first ordering, single-layout skip), the injectable `new_with`, and `new` |
| `src/flows/mod.rs` | Declares `menu` and `tab` |
| `src/config.rs` | `TabLayout.label`, `TabLayout.icon`, `TabLayout::menu_label()`, two new validations |
| `src/main.rs` | `Command::Tab`, `channel()` → `Channel::Notification("New tab")`, `subcommand_path()` → `"tab new"`, router arm, parse/channel test rows |
| `herdr-plugin.toml` | `[[actions]] id = "new-tab"` and `[[panes]] id = "new-tab"` running `./bin/workbench tab new` |
| `README.md` | Actions table row, keybinding table row, the two new layout keys |
| `config.example.toml` | Document `label` and `icon` under the layout keys |
| `bin/workbench` | Rebuilt via `zsh scripts/build.zsh` and committed |

### Menu contract

`choose_layout` follows `choose_agent`'s existing pattern exactly: options are rendered
labels, `GumMenu::padded` indents them, the selection returns padded, `strip_pad` recovers
it, and the row maps back to a layout by its rendered label. Uniqueness of rendered labels
is enforced at config load, so the reverse lookup cannot be ambiguous.

## Requirements & acceptance

| # | Requirement | Verified by |
| --- | --- | --- |
| R1 | Menu primitives live in `src/flows/menu.rs`, with no behaviour change | `cargo test`, `cargo clippy -- -D warnings` |
| R2 | `[[tab_layouts]]` accepts optional `label` and `icon`; the menu label falls back to `name` | config unit tests |
| R3 | Empty `label`/`icon`, and two layouts rendering the same menu label, are named load-time errors | config unit tests |
| R4 | `agent popup` behaves identically after the `popup_with_layout` extraction | existing popup tests, unchanged and passing |
| R5 | `tab new` lists layouts default-first, otherwise in config order | `tab.rs` unit test with a fake `Menu` |
| R6 | A single configured layout skips the menu | `tab.rs` unit test asserting zero menu calls |
| R7 | Cancelling the layout menu returns `Outcome::Cancelled` and the flow issues no socket call | `tab.rs` unit test driving `new_with` with a `FakeClient` |
| R8 | `tab new` parses, routes to `Channel::Notification("New tab")`, and reports as `tab new` | `main.rs` tests |
| R9 | Choosing a non-default layout builds *that* layout's tab | `tab.rs` unit test asserting the `FakeClient` call sequence, then confirmed by a manual run in Herdr after the rebuild |
| R10 | README and `config.example.toml` describe the action and the two new keys | final review reads both |
| R11 | `bin/workbench` is rebuilt from the final source | final review runs `zsh scripts/build.zsh` |

## Bucketing

Single `work/` bucket. One linear feature with no independent tracks worth separating.

## Task index

| # | Task | Depends on | Blocks |
| --- | --- | --- | --- |
| 01 | `work/01-menu-module.md` — extract menu primitives into `src/flows/menu.rs` | none | 03, 04, 07 |
| 02 | `work/02-layout-menu-labels.md` — `label`/`icon` fields, `menu_label()`, validations | none | 04, 06, 07 |
| 03 | `work/03-popup-with-layout.md` — extract `agent::popup_with_layout` | 01 | 04, 07 |
| 04 | `work/04-tab-flow.md` — `src/flows/tab.rs`: `choose_layout` + `new` | 01, 02, 03 | 05, 07 |
| 05 | `work/05-cli-and-manifest.md` — `tab new` in the router, action + pane in the manifest | 04 | 06, 07 |
| 06 | `work/06-docs.md` — README + `config.example.toml` | 02, 05 | 07 |
| 07 | `work/07-final-review.md` — integration, full suite, rebuild `bin/workbench` | 01–06 | — |

Tasks 01 and 02 are independent and can run in the same wave. Everything else is a chain.
Tasks 01 and 03 both edit `agent.rs`; 03 depends on 01 so they never run together.

## Eval rubric (shared)

Pass `> 4.0` on the 0–5 scale. `Correctness < 4` is an automatic veto.

| Dimension | Weight |
| --- | --- |
| Correctness | ×3 |
| Test coverage | ×2 |
| Interface & readability | ×1 |
| Assumptions & docs | ×1 |

Task 07 scores integration axes instead: Integration, Meets the goal, Consistency, No
regressions.

## Verification

```zsh
cargo test
cargo clippy -- -D warnings
zsh scripts/build.zsh
```

Then in Herdr: trigger the `new-tab` action, confirm the layout menu lists every
configured layout with the default first, pick a non-default one, and confirm the tab is
built from that layout.

## Failure modes & rollback

| Risk | Mitigation |
| --- | --- |
| Stale `bin/workbench` — the linked checkout keeps running the old binary | Task 07 rebuilds and commits it; R11 gates it |
| The `menu.rs` extraction silently changes menu rendering | Task 01 is a pure move: no signature, string, or logic edits; existing popup tests must pass untouched |
| Two layouts rendering the same menu label make one unselectable | Load-time validation (R3), mirroring the agent-label check |
| A half-built tab if the popup fails mid-construction | Unchanged: `create_popup_tab` already closes the tab and reports through `FlowError::prefixed` |

Rollback is `git revert` of the feature commits plus a rebuild of `bin/workbench` from the
reverted source. No state file, registry, or config format is migrated, so nothing outside
the repository needs undoing; a `config.toml` carrying the new `label`/`icon` keys would
fail to load against the old binary and needs those two keys removed.

## Open questions

None.
