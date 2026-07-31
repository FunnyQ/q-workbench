# AGENT-02: `workbench agent popup`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: agent/01, foundation/05
> **Blocks**: polish/02
> **Status**: todo

## Goal

The popup entry point: collect every choice first, then build the tab and its three
panes, cleaning up completely if anything fails partway.

## Files to create / modify

- `src/flows/agent.rs` (modify) — add the popup entry point
- `src/main.rs` (modify) — wire `agent popup [--worktree]`

## Implementation notes

### Shape

This runs *inside a popup pane*, so it owns no pane of its own to launch into. It
collects all choices through the shared flow, then creates the tab and panes over the
socket, and finally focuses the tab. The exact ten-call sequence, with every
parameter, is inlined in the parity contract — reproduce it method for method.

### The popup cwd trap

A plugin popup starts with the **plugin directory** as its cwd, and that directory is
itself a git checkout. A bare `git rev-parse` there resolves to the plugin, not the
user's workspace. Before anything reads the working directory — worktree discovery
and the project-directory fallback both do — adopt the invoking pane's cwd:

1. `focused_pane_cwd` from `HERDR_PLUGIN_CONTEXT_JSON`
2. failing that, `pane.get` on `HERDR_ACTIVE_PANE_ID` and read its `cwd`
3. change the process working directory to it if it exists

This is a real regression that was fixed once already; the existing test at
`tests/new-agent-popup.test.zsh:101-129` pins both paths.

### Workspace

Pass `workspace_id` to `tab.create` only when `HERDR_WORKSPACE_ID` is set and
non-empty. Omit the field otherwise; do not send an empty string.

### Cleanup

Once the tab exists, **every** subsequent failure must close the tab and notify. The
zsh version's `cleanup_tab` shows the title `Agent tab failed` with the body
`The incomplete tab was closed.` Keep both strings. A partially built tab left on
screen is worse than no tab.

Before the tab exists, a cancelled menu must leave no trace at all — the existing test
asserts an empty command log for that case.

Structure this so the cleanup cannot be forgotten: create the tab, then run the rest
inside a closure or a helper returning `Result`, and handle the error in one place.

A later task moves the *reporting* out of this flow: closing the tab stays here, but
the notification is emitted once by a top-level handler, with the cause appended to the
preserved sentence. Both strings survive that move. Keep the close and the report
separable so that change is a small one — do not interleave them.

### Sizing

The popup measures its own viewport from `COLUMNS`/`LINES`, falling back to `tput`.
Unlike the in-pane launcher it does **not** consult `pane.layout` — it is not a
managed pane. Pass the measured size into the shared flow.

## Acceptance criteria

- [ ] The ten-call sequence in the parity contract is reproduced exactly, in order,
      with the same parameters — including `Q_NO_BANNER` on the tab and the first
      split but not the second.
- [ ] `--worktree` runs the worktree step first; without it, no worktree step.
- [ ] When worktree creation fails, the choice is normalised before any pane is created:
      the repository toplevel is used as `cwd`, and neither the tab nor the pane label
      carries a branch suffix.
- [ ] The tab cwd follows `focused_pane_cwd`, with `HERDR_ACTIVE_PANE_ID` as fallback.
- [ ] `workspace_id` is sent only when `HERDR_WORKSPACE_ID` is non-empty.
- [ ] Cancelling any menu creates no Herdr resource.
- [ ] Any failure after `tab.create` closes the tab and notifies with the two strings
      above.
- [ ] The launch command is submitted with `keys: ["enter"]` and is shell-quoted.

## Verification

- [ ] `cargo test` — with `FakeClient`, assert the full ten-call sequence and every
      parameter, mirroring the assertion block in `tests/new-agent-popup.test.zsh`
- [ ] `cargo test` — a cancelled menu records zero calls
- [ ] `cargo test` — a failure injected at each step after `tab.create` results in a
      `tab.close` plus a `notification.show`
- [ ] `cargo test` — the bypass flag is absent by default and present when configured,
      pinning both states as the zsh test does
- [ ] `cargo test` — configured extra args map one TOML array element to one argv
      entry: `["--search", "--profile", "work"]` yields three, and
      `["--profile work"]` yields one entry containing a space
- [ ] Manual through the linked dev plugin: run new-agent and new-worktree-agent once
      each, confirm the 3-pane layout and the tab label
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Call sequence or parameters differ; or a failure leaves a half-built tab | Sequence right but the popup cwd trap or the `Q_NO_BANNER` asymmetry is wrong | Exact sequence, correct cwd adoption, complete cleanup |
| Test coverage | ×2 | No sequence assertion | Sequence asserted, failure paths not | Sequence, every failure injection point, cancellation, and both bypass states |
| Interface & readability | ×1 | Cleanup duplicated at each call site | Centralised but tangled with menu logic | Menu decision and tab construction cleanly separated; one cleanup path |
| Assumptions & docs | ×1 | Popup cwd trap uncommented | Mentioned without the reason | The plugin-dir-is-a-git-checkout reason written down at the adoption site |

## Out of scope

- The in-pane launcher and restart — separate tasks, sharing the same decision module.
- Consistent notification coverage across all flows — a later polish task sweeps.
