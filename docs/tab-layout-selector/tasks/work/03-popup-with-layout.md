# WORK-03: Split the agent popup into a reusable half

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: work/01
> **Blocks**: work/04, work/07
> **Status**: todo

## Goal

Extract the part of `agent::popup` that runs the menus and builds the tab into
`popup_with_layout`, so a second entry point can call it with an already-chosen layout,
with no change to what `agent popup` does.

## Files to create / modify

- `src/flows/agent.rs` (modify) — add `popup_with_layout`, reduce `popup` to a wrapper,
  and widen `popup_viewport` to `pub(crate)`

## Implementation notes

### The split

`popup` currently does six things in a fixed order: load config, resolve the requested
layout, adopt the invoking pane's cwd, read the viewport, run `choose_agent`, realise any
worktree, create the tab. The first two are the caller's business; the rest is shared.

Target shape:

```rust
/// Collect a popup decision, then create and focus its tab.
pub fn popup(client: &dyn HerdrClient, worktree: bool, requested_layout: Option<&str>) -> FlowResult {
    // Config first: adopting the invoking pane's cwd queries Herdr, and a broken config
    // must be reported before the first socket call.
    let config = Config::load().context("failed to load config")?;
    let layout = resolve_layout(&config, requested_layout)?;
    popup_with_layout(client, &config, layout, worktree)
}

/// The popup flow from the invoking pane's cwd onwards, for a layout the caller has
/// already resolved.
pub(crate) fn popup_with_layout(
    client: &dyn HerdrClient,
    config: &Config,
    layout: &TabLayout,
    worktree: bool,
) -> FlowResult {
    adopt_invoking_pane_cwd(client)?;
    let cwd = std::env::current_dir().context("failed to read popup working directory")?;
    let (cols, lines) = popup_viewport();
    let Some(mut choice) = choose_agent(config, layout, &cwd, worktree, None, cols, lines, None)?
    else {
        return Ok(Outcome::Cancelled);
    };

    if let Some(branch) = choice.branch.clone() {
        let repo_root = RealGit.toplevel(&cwd).unwrap_or_else(|| cwd.clone());
        if realise_worktree(&repo_root, &branch).is_none() {
            choice = without_worktree(choice, &repo_root);
        }
    }

    create_popup_tab(client, layout, &choice, nonempty_env("HERDR_WORKSPACE_ID"))?;
    Ok(Outcome::Done)
}
```

Move the body statement for statement. Do not reorder, do not merge steps, do not change an
error message.

### Why `worktree` stays a parameter

`popup_with_layout` keeps the `worktree` flag even though the new caller will always pass
`false`. Dropping it would force `popup` to keep its own copy of the whole body for the
worktree case, which is the duplication this task removes.

### The viewport helper

`popup_viewport()` is the fallback chain a popup uses to size its menus
(`flows::terminal_size()`, then `COLUMNS`/`LINES`, then `tput`, then 80). A second flow
will draw menus at the same size, so widen it to `pub(crate)`:

```rust
pub(crate) fn popup_viewport() -> (u16, u16);
```

Its body does not change. This is the only other edit this task makes.

### Ordering that must survive

- Config load stays in `popup`, before that flow issues its first request.
  `adopt_invoking_pane_cwd` issues `pane.get`, so a config error raised after it would
  arrive after the flow's first socket call — the exact inversion the existing comment
  warns about. Keep that comment on the config load. (The protocol-guard `ping` in `main`
  runs earlier still, for every notifying command; that is unrelated and unchanged.)
- `adopt_invoking_pane_cwd` must run before `std::env::current_dir()`. It changes the
  process cwd, and a popup starts in the plugin checkout, not the project.
- The viewport is read before the first menu draws.
- The worktree is only realised after a choice comes back, so cancelling leaves no
  directory and no branch behind.

### Visibility

`popup_with_layout` is called from another module in this crate but never from outside it:
`pub(crate)`, not `pub`.

### Tests

No new tests are required and no existing popup test should need editing: they drive
`popup` through a `FakeClient` and a scripted config, and the call sequence is unchanged.
If a test does need editing to compile, that is a signal the extraction changed behaviour —
fix the extraction, not the test.

## Acceptance criteria

- [ ] `agent::popup_with_layout(client, &Config, &TabLayout, worktree: bool) -> FlowResult`
      exists and is `pub(crate)`.
- [ ] `agent::popup` keeps its exact signature and delegates to it after loading the config
      and resolving the layout.
- [ ] `agent::popup_viewport` is `pub(crate)` with an unchanged body.
- [ ] The statement order of the extracted body is identical to before.
- [ ] The config-first comment still sits on the config load in `popup`.
- [ ] No existing popup test was edited.

## Verification

- [ ] `cargo test` passes, including `popup_reproduces_the_exact_ten_call_sequence`,
      `popup_cancelled_choice_makes_zero_calls`, and
      `popup_failure_at_every_post_create_step_closes_and_returns_metadata`.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `git status --short -- src/flows/agent.rs` shows the file dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Steps reordered, config load moved past the first socket call, or an error message changed | Compiles and passes, but a step was merged or a comment dropped | Body moved statement for statement, ordering and messages intact |
| Test coverage | ×2 | A popup test was edited or deleted to make the build pass | Tests pass but a failure-path test was weakened | Every popup test unchanged and green |
| Interface & readability | ×1 | Made `pub`, or takes an owned `Config` | Extra parameters with no caller | `pub(crate)`, borrowed `Config` and `TabLayout`, `worktree` justified |
| Assumptions & docs | ×1 | Ordering reasons lost | Comments kept but detached from their code | Ordering comments sit on the statements they explain |

## Out of scope

- Any new caller. This task only exposes the seam.
- Changing `choose_agent`, `create_popup_tab`, or `resolve_layout`.
- Extracting a similar seam from `agent::launch` or `agent::inject`. Neither is needed here.
