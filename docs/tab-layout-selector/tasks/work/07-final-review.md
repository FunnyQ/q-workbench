# WORK-07: Final review, rebuild, and manual run

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: work/01, work/02, work/03, work/04, work/05, work/06
> **Status**: todo
> **Final review**: true

## Goal

Confirm the whole change composes — menu module, layout labels, popup seam, tab flow, CLI,
manifest, docs — then rebuild the committed binary and drive the new action by hand twice:
once cancelling, once selecting a non-default layout.

## Files to create / modify

- `bin/workbench` (modify) — rebuilt from the final source by `zsh scripts/build.zsh`
- Any file above that needs a fix found during review — repair in place, do not re-plan

## Implementation notes

This task reviews integration, not individual tasks. Each earlier task already passed its
own gate. Look for what only shows up once everything is present.

### Integration checks

1. **The chain actually connects.** Follow it in the source: the `tab new` clap leaf →
   the router arm → `flows::tab::new` → `choose_layout` → `agent::popup_with_layout` →
   `create_popup_tab`. Confirm no step was stubbed or left calling the old path.
2. **One config load per invocation.** `flows::tab::new` loads the config; the router arm
   and `popup_with_layout` must not load it again.
3. **Cwd adopted once, and only after a layout is chosen.** `popup_with_layout` adopts the
   invoking pane's cwd as its first step. Confirm the tab flow does not also call it, and
   that the layout menu draws before it — cancelling at the layout menu returns before
   `popup_with_layout` is reached, so it adopts nothing and issues no request.
4. **The menu module points one way.** `src/flows/menu.rs` must not reference
   `crate::flows::agent` or `crate::flows::tab`.
5. **`agent popup` is untouched in behaviour.** The existing popup tests must still pass
   unedited, and the three existing pane commands in `herdr-plugin.toml` must be byte-identical
   to what they were before this plan:

   | Pane id | Command | Layout it resolves to |
   | --- | --- | --- |
   | `agent` | `["./bin/workbench", "agent", "popup"]` | `default_tab_layout` |
   | `worktree-agent` | `["./bin/workbench", "agent", "popup", "--worktree"]` | `default_tab_layout` |
   | `assistant` | `["./bin/workbench", "agent", "popup", "--layout", "personal-assistant"]` | `personal-assistant` |

   Check the commands with `git diff -- herdr-plugin.toml`: the only changes in that file
   must be the two added entries. Check the resolution rule by reading `agent::popup` —
   `resolve_layout(config, requested)` still falls back to `config.default_tab_layout` when
   `requested` is `None`.
6. **Validation covers what the menu assumes.** The layout reverse lookup relies on unique
   rendered rows and on `default_tab_layout` naming a real layout. Both must be enforced at
   config load, not at menu time.
7. **Manifest halves agree.** The action's `--entrypoint` equals the pane `id`, and the
   pane command is `["./bin/workbench", "tab", "new"]`.
8. **Docs match the code.** Every claim in `README.md` and `config.example.toml` about the
   new action and the two new keys is true of the shipped flow.
9. **Nothing unrelated moved.** `src/flows/layout.rs` (pane split ratios), the pickers, the
   registries, and the restart worker should be untouched.

### Rebuild

`herdr-plugin.toml` entries call the committed `bin/workbench`, and a linked checkout runs
that artifact. Skipping the rebuild is the usual cause of code and behaviour disagreeing.
Run it last, after every source fix:

```zsh
zsh scripts/build.zsh
```

Then confirm the rebuilt binary carries the new command:

```zsh
./bin/workbench tab --help
```

### Manual run

Herdr must be running for this.

**Set up a config that can show the difference.** The checks below need at least two
layouts, one of them non-default and carrying `label` and `icon`, with panes that differ
visibly from the default layout's. The user config lives at
`~/.config/herdr/plugins/config/q.workbench/config.toml`, and the plugin process inherits
Herdr's environment rather than the reviewing shell's, so `Q_WORKBENCH_LOCAL_CONFIG` does
not help here. Edit the real file, and restore it afterwards:

1. Record whether the file exists at all: `ls -l ~/.config/herdr/plugins/config/q.workbench/config.toml`.
   Write the answer down — the two branches restore differently.
2. If it exists, back it up and hash it before touching anything:
   `cp ~/.config/herdr/plugins/config/q.workbench/config.toml ~/.config/herdr/plugins/config/q.workbench/config.toml.review-backup`
   then `shasum -a 256 ~/.config/herdr/plugins/config/q.workbench/config.toml.review-backup`
   and keep the digest.
3. Edit the file so it defines at least two `[[tab_layouts]]` entries. Keep the current
   default as one. Add a second, non-default one with a distinct `name`, a `label`, an
   `icon`, and a pane set that looks different on screen — a single agent pane with no side
   panes makes a wrong layout obvious at a glance.
4. Run the checks below. **Restore first if anything goes wrong** — a failed check, an
   interrupted session, or a decision to stop. The review is never a reason to leave the
   temporary config installed.
5. Restore, without deleting the backup first:
   - The file existed: `cp …/config.toml.review-backup …/config.toml`, then
     `shasum -a 256 …/config.toml` and confirm the digest matches the one recorded in
     step 2. Only then delete the backup.
   - The file did not exist: delete the file you created, and confirm with `ls` that it is
     gone.

**Invocation one — cancel.** Trigger the `new-tab` action from Herdr's action list:

- The layout menu lists every configured layout, `default_tab_layout` first.
- The layout carrying `label` and `icon` renders as that row, not as its `name`.
- Escape closes the popup silently, with no notification and no new tab.

**Invocation two — select.** Trigger the action again:

- Choose the non-default layout.
- No worktree menu appears at any point.
- The tab that opens is built from the chosen layout: its panes, its pane labels, and its
  tab label, not the default layout's.

The manual run is a gate, not a courtesy. If Herdr is not available in this environment,
set this task's `Status` to `blocked`, state that Herdr was unavailable, and leave the
manual criteria unticked. Do not mark the task `done` with them unticked, and do not tick
them from reasoning about the code.

The automated half of the same claim — that choosing a non-default layout builds *that*
layout's tab — is already covered by a unit test in `src/flows/tab.rs` driving the flow
against a `FakeClient`. Confirm that test exists and passes. It is what keeps the plan
verifiable when the manual run cannot happen; the manual run is what proves it against the
real Herdr.

### Fixes

Repair anything found, in place, in the file that owns it. Re-run the suite after each fix.
Do not expand scope: an unrelated problem noticed here gets reported, not fixed.

## Acceptance criteria

- [ ] The full call chain from the `tab new` leaf to `create_popup_tab` is present and
      connected.
- [ ] Exactly one config load per `tab new` invocation, and exactly one cwd adoption once a
      layout is selected — none at all when the layout menu is cancelled, which also issues
      no socket call from the flow.
- [ ] `src/flows/menu.rs` references neither `crate::flows::agent` nor `crate::flows::tab`.
- [ ] The existing agent-popup tests pass unedited.
- [ ] `git diff -- herdr-plugin.toml` shows only the two added entries: the three existing
      pane commands and their `--layout` arguments are byte-identical, and `resolve_layout`
      still falls back to `default_tab_layout`.
- [ ] Unique rendered layout rows and a valid `default_tab_layout` are both enforced at
      config load.
- [ ] The manifest action's `--entrypoint` matches the pane id, and the pane runs
      `./bin/workbench tab new`.
- [ ] README and `config.example.toml` make no claim the code does not honour.
- [ ] `bin/workbench` is rebuilt from the final source and `./bin/workbench tab --help`
      lists `new`.
- [ ] A unit test in `src/flows/tab.rs` proves that selecting a non-default layout builds
      that layout's tab, and it passes.
- [ ] Both manual invocations ran against a config with two or more layouts, and matched
      every expectation listed for them. If Herdr is unavailable, this task's `Status` is
      `blocked` with that reason, not `done`.
- [ ] The user config file is restored: either its SHA-256 matches the digest recorded
      before the edit, or — if it did not exist before — it does not exist now.

## Verification

- [ ] `cargo test` passes with zero failures.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `zsh scripts/build.zsh` succeeds.
- [ ] `./bin/workbench tab --help` prints the `new` subcommand.
- [ ] `grep -rn "crate::flows::agent\|crate::flows::tab" src/flows/menu.rs` returns nothing.
- [ ] `git status --short -- bin/workbench` shows the rebuilt binary dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Integration < 4 is an automatic veto. A task left `blocked` is not scored and does not pass — score it only once every gate has run.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Integration | ×3 | The chain is broken, or the rebuilt binary lacks the command | Compiles and runs, but the config is loaded twice or the cwd adopted twice | Every seam connects, one load, one adoption, menu before the flow's first socket call |
| Meets the goal | ×2 | The action does not open a tab from a chosen layout, or manual criteria were ticked without a run | One invocation only, or the ordering or single-layout skip is wrong | Both invocations ran against a multi-layout config and matched every expectation, with the automated layout test green and the config restored |
| Consistency | ×1 | Docs contradict the code | Minor drift between README and the example config | Code, manifest, README, and example config all agree |
| No regressions | ×1 | An existing action or test broke | Tests pass but an agent-popup test was edited to make them pass | Suite green, the agent-popup tests unedited, only the planned menu-test move and config-test updates present, existing actions unchanged |

## Out of scope

- CHANGELOG entry and version bump. A release step owns both.
- Fixing unrelated problems found while reviewing. Report them instead.
- Cross-platform builds. The committed artifact stays macOS arm64.
- Any new feature suggested by the review — record it, do not build it.
