# AGENT-03: `workbench agent launch` and `agent inject`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: agent/01, foundation/05
> **Blocks**: agent/04, picker/02, polish/02, polish/06
> **Status**: todo

## Goal

The in-pane entry point: run every menu at full pane width, split the side panes last,
and replace the process with the harness via a real `execvp`.

## Files to create / modify

- `src/flows/agent.rs` (modify) — add `launch` and `inject`
- `src/main.rs` (modify) — wire `agent launch` and `agent inject`

## Implementation notes

### `exec` is the whole point

This subcommand must end with
`std::os::unix::process::CommandExt::exec()`, which performs a real `execvp` and never
returns on success. **Do not spawn the harness as a child process.**

The reason is restart-in-place. The launcher is injected into a pane via
`pane.send_input`, so it runs as a child of the pane's interactive shell. `exec`
replaces the *launcher* process, not the shell — so killing the agent's foreground
process group later drops the pane back to its prompt instead of destroying the pane.
A wrapper process left alive breaks that, and would also likely stop Herdr detecting
the pane as an agent pane. This constraint is the reason the whole rewrite is in Rust
rather than TypeScript.

`exec()` returns only on failure. Treat that return as fatal: notify with the reason
and exit non-zero.

Change the working directory to the resolved project directory and `clear` the screen
immediately before the `exec`, as the zsh version does.

### Ordering is load-bearing

Run every menu at the pane's **full width**, and defer both `pane.split` calls to the
very end. Splitting earlier resizes the pane mid-menu, which breaks the centering; and
deferring lets a chosen worktree drive `cwd` for all three panes rather than only the
agent's. This ordering is deliberate and must not be "tidied".

`--no-layout` skips the two splits entirely. That is what the restart path uses, since
the yazi and term panes already exist — but it is a public option, not a restart
marker. A separate hidden `--restart` flag carries that meaning; accept it, and for now
ignore it. A later task gives it behaviour.

### Sizing

Take the viewport from `pane.layout` for this pane, not `tput`: a restarted pane can
briefly report the old size while it settles, and Herdr's layout is the source of
truth. Fall back to `COLUMNS`/`LINES`, then `tput`, in that order — the parity
contract records the full chain.

### Naming

Rename the pane to the chosen label. Rename the tab too, but only when `--tab` was
given — the project picker deliberately omits it to keep its own tab name.

### `agent inject`

The thin wrapper that puts the launcher into an existing pane, used by the project
picker and by any external shell function. It renames the pane to the agent label from
the parity contract, then sends the launcher command into the pane via
`pane.send_input` with `keys: ["enter"]`.

Build that command from `std::env::current_exe()` and the named flags, all passed
through the shell-quoting helper. The old convention of empty quoted positional slots
is gone — named flags mean an omitted value simply is not passed.

## Acceptance criteria

- [ ] `agent launch` ends in `CommandExt::exec()`; no child process wraps the harness.
- [ ] A failed `exec` notifies with the reason and exits non-zero.
- [ ] Both splits happen after every menu, with the parameters in the parity contract.
- [ ] `--no-layout` skips both splits and performs no rename of the side panes.
- [ ] The tab is renamed only when `--tab` is given.
- [ ] Viewport size comes from `pane.layout`, with the documented fallback chain.
- [ ] The working directory is the chosen worktree when one was selected, otherwise
      the repo toplevel, otherwise the current directory.
- [ ] When worktree creation fails, the choice is normalised before any pane is created:
      the repository toplevel is used as `cwd`, and neither the tab nor the pane label
      carries a branch suffix.
- [ ] `agent inject` renames the pane and sends a correctly quoted launcher command
      with `keys: ["enter"]`.

## Verification

- [ ] `cargo test` — with `FakeClient`, assert the split sequence and parameters, and
      that `--no-layout` produces no split calls
- [ ] `cargo test` — assert the tab rename happens only with `--tab`
- [ ] `cargo test` — `agent inject` produces one `pane.rename` and one
      `pane.send_input` whose text round-trips through a shell back to the intended argv
- [ ] Manual in a **scratch tab**: run `agent launch` into a pane and confirm the
      harness replaces the launcher — check that the pane's process tree has no
      `workbench` process left
- [ ] Manual: confirm menus stay centered throughout, with no visible resize until the
      final split
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Spawns the harness as a child instead of `exec`, or splits before the menus | Uses `exec` but the fallback sizing chain or the `--tab` rule differs | Real `execvp`, splits deferred, every parameter and rule matches |
| Test coverage | ×2 | No split assertions | Splits asserted, `--no-layout` not | Splits, `--no-layout`, tab-rename conditionality, inject quoting, plus the live no-wrapper-process check |
| Interface & readability | ×1 | Menu logic duplicated from the popup path | Shares the module but with special-casing | Shares the decision module cleanly; this file is layout and `exec` only |
| Assumptions & docs | ×1 | Deferred-split ordering uncommented | Noted without the reason | Both reasons — mid-menu resize and worktree-driven cwd — written down, plus why `exec` cannot become `spawn` |

## Out of scope

- Killing a running agent and re-injecting — the restart task.
- Remembering the last harness and model — a later polish task.
