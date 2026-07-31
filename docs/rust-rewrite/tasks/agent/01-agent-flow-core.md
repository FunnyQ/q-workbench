# AGENT-01: Shared agent menu flow

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/03
> **Blocks**: agent/02, agent/03
> **Status**: todo

## Goal

One module that runs the worktree, harness, model and usage menus and returns a
resolved launch decision — the single implementation both agent entry points use.

## Files to create / modify

- `src/flows/agent.rs` (new) — the shared flow, menus, and worktree handling

## Implementation notes

### Why this task exists

`scripts/new-agent-popup.zsh:53-187` and `scripts/agent-launcher.zsh:75-205` are two
implementations of the same flow, differing only in whether the tab exists yet.
Collapsing them is the single largest maintainability win of the rewrite. Everything
that is *not* about menus — creating tabs, splitting panes, `exec`ing — stays out of
this module.

### The returned decision

```rust
pub struct AgentChoice {
    pub label: String,          // pad-stripped, glyph kept, branch appended if any
    pub project_dir: PathBuf,   // the worktree if one was chosen, else the repo toplevel or cwd
    pub branch: Option<String>,
    pub launch: Vec<String>,    // argv, ready for exec or for build_command
    pub harness: String,             // the pad-stripped harness menu label
    pub model_label: Option<String>, // the pad-stripped model menu label; None for codex/opencode
}
```

The last two fields are the **menu labels**, not resolved values. They exist so a
caller can record what was chosen; the model label is what resolves through the config
maps, so storing the resolved model value instead would break when a user renames a
menu entry.

Every menu returning "cancelled" propagates as `Ok(None)` from the flow, not an error.
Cancellation is normal and must produce no side effects and no notification.

### Menu order

**harness → model → usage.** This is a sanctioned deviation for the in-pane launcher,
which previously asked harness → usage → model. The model menu only appears for the
claude harness.

The optional worktree step runs *before* the harness menu when worktree mode is on.

All exact labels, glyphs and codepoints are in the parity contract. So are the launch
command rules per harness and the worktree directory naming, branch filtering and
fallback behaviour.

### Rendering

Menus shell out to `gum`. `Command::output()` captures the selection from stdout while
`gum` draws on the TTY; a non-zero exit means cancelled.

The centering machinery must survive: a banner drawn with `gum style --border rounded
--padding '1 3' --width <w>`, then each of its lines printed at a computed left
margin. Wrapping an already-styled multiline banner in another `gum` call offsets its
border lines, which is why the zsh version prints the lines itself. Do the same.

Take the viewport size as parameters (`cols`, `lines`) rather than measuring inside
this module — the popup and the in-pane launcher obtain them differently, and that
difference is load-bearing (see the sizing rule in the parity contract).

Menu options carry a leading pad for centering. Strip it from the selection before
use, but **keep the glyph** — the stripped label becomes the pane and tab label.

### Worktree step — select first, create last

Follows the parity contract: prune, offer only branches not already checked out, one
field that both filters and names, auto-name on empty, reuse an existing directory,
`-b` for a new branch, and fall back to no worktree if `git worktree add` fails rather
than aborting the flow.

**Creation is deferred.** The step still runs first, because the chosen branch drives
the directory that every pane is born in — but this module only *selects*. It returns
the branch and the intended directory; the caller creates the worktree after the flow
returns a choice. That is what makes "cancelling has no side effect" true: today,
cancelling at the harness menu leaves an orphaned worktree behind, because
`git worktree add` already ran.

Concretely: the module computes `branch` and `project_dir` and returns them. Creating
`project_dir` — including the reuse check, the `-b` decision, and the
fall-back-to-no-worktree behaviour on failure — happens in the caller, once. Expose it
as a separate function in this module so both entry points share it and neither
reimplements the rules:

```rust
/// Create or reuse the worktree for a chosen branch.
/// Returns None when `git worktree add` failed, meaning: proceed without one.
pub fn realise_worktree(repo_root: &Path, branch: &str) -> Option<PathBuf>
```

**When it returns `None`, the choice must be normalised before use.** Otherwise the
caller would split panes into a directory that does not exist and label the tab with a
branch that was never created. Provide that normalisation here so neither entry point
reimplements it:

```rust
/// Strip the worktree from a choice whose creation failed.
/// - project_dir → the repository toplevel (or the original cwd outside a repo)
/// - branch      → None
/// - label       → the usage label alone, with the two-space branch suffix removed
pub fn without_worktree(choice: AgentChoice, repo_root: &Path) -> AgentChoice
```

The zsh version achieved the same by clearing `worktree_dir` and `branch` before the
label was ever composed. Because the Rust flow composes the label earlier, the suffix
has to be removed explicitly — so normalise by rebuilding the label from its parts, not
by trimming the composed string.

Guard the whole selection step on actually being inside a work tree.

### Label composition

Usage label, then — when a worktree was chosen — two spaces and the branch name. This
is what keeps parallel worktree tabs distinguishable.

## Acceptance criteria

- [ ] The flow runs worktree (optional) → harness → model (claude only) → usage and
      returns one `AgentChoice`.
- [ ] Cancelling at any menu returns "no choice" with no side effect and no
      notification — specifically, **no worktree directory or branch is created**.
- [ ] The flow only selects a branch; `realise_worktree` is a separate function the
      caller invokes after a choice is returned.
- [ ] A `fixed_usage` input skips the usage menu and uses that label verbatim.
- [ ] Labels, glyphs and the `let me write…` free-text path match the parity contract,
      including keeping the glyph after pad-stripping.
- [ ] Launch commands match the per-harness rules, including `CCR` dispatching to
      `ccr code` with no model flag and no extra args.
- [ ] Extra args arrive as separate argv entries, and an argument containing a space
      stays one entry.
- [ ] Worktree selection reproduces every rule in the parity contract, including
      falling back to no worktree when `git worktree add` fails.
- [ ] A failed creation yields a choice with `branch` cleared, `project_dir` set to the
      repository toplevel, and the branch suffix removed from the label — so no caller
      can split a pane into a directory that was never created.
- [ ] The module creates no tabs, splits no panes, and never `exec`s.
- [ ] `AgentChoice` carries the harness label and the optional model label alongside
      the assembled argv.

## Verification

- [ ] `cargo test` — launch-command assembly for each harness and each default model
      label, with and without extra args, including a multi-word extra arg
- [ ] `cargo test` — label composition: pad stripped, glyph kept, branch appended
- [ ] `cargo test` — worktree directory naming, including a branch containing a slash
- [ ] `cargo test` — `without_worktree` on a choice whose creation failed: `branch` is
      `None`, `project_dir` is the repository toplevel, and the label equals what the
      same usage selection produces with no worktree at all
- [ ] Manual through the linked dev plugin: run the flow once per harness and confirm
      the menus render centered and in the new order
- [ ] `cargo test` — in a fixture git repo, cancel at the harness, model and usage
      menus in turn and assert `git worktree list` is unchanged and no new branch exists
- [ ] Manual: cancel at each of the four menus and confirm nothing was created
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A glyph, label or launch command differs; or cancellation has side effects | Menus right but a worktree edge case (slash in branch, add failure) drifts | Every label, glyph, launch rule and worktree rule matches |
| Test coverage | ×2 | No assembly tests | Happy-path assembly only | Every harness, every model label, extra-arg splitting, label composition, and all four cancel points |
| Interface & readability | ×1 | Menus and Herdr calls mixed together | Separated but the viewport size measured inside | Pure decision module; viewport passed in; one `AgentChoice` out |
| Assumptions & docs | ×1 | Centering trick uncommented | Mentioned without the reason | The nested-`gum` offset problem and the pad-strip-but-keep-glyph rule both explained |

## Out of scope

- Building the tab layout, injecting into a pane, or `exec`ing the harness — the two
  entry-point tasks do that.
- The "use last combination" entry — a later polish task adds it.
