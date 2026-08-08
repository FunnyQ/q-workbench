# ADR-0003: Popup cwd resolves from Herdr's active-pane data, not process cwd

- Status: Accepted
- Date: 2026-07-22

## Context

Herdr opens a plugin popup pane with the plugin's own install directory as its cwd (`~/.config/herdr/plugins/github/q.workbench-<hash>`), and that install directory is itself a git checkout. Because of that, `git rev-parse --show-toplevel` run inside a popup script such as `new-agent-popup.zsh` does not fail — it silently resolves to the plugin's own repo instead of erroring, so there is no obvious signal that the wrong directory was used. A new tab opened from the popup then lands in the plugin's directory instead of the user's workspace.

A first fix attempt replaced only the `project_dir=$(git rev-parse --show-toplevel …)` assignment with a value derived from the calling pane. That was tried and found insufficient: `new-agent-popup.zsh` reads `$PWD` in two places, and the first is much earlier — worktree-mode branch/worktree discovery (`git rev-parse --is-inside-work-tree`, `git worktree list`, `git worktree add`) runs while menu choices are still being collected, well before `project_dir` is ever computed. Patching only the later line left worktree mode still reading the wrong directory.

The test suite hit a related isolation gap: `tests/new-agent-popup.test.zsh` avoided the bug by `cd`-ing into a temp directory that isn't a repo, but without also explicitly clearing `HERDR_ACTIVE_PANE_ID`, the developer's own active pane could leak into the test — the same category of isolation gap as needing `Q_WORKBENCH_LOCAL_CONFIG` forced to `/dev/null` in tests.

## Considered alternatives

- Patch only the `project_dir=$(git rev-parse --show-toplevel …)` line to use the calling pane's cwd. Tried and rejected as insufficient: worktree-mode discovery consumes `$PWD` earlier in the script, before `project_dir` is ever computed, so this fix left worktree mode broken.
- Rely on ambient `$PWD` / process cwd in general. Rejected: a popup pane's cwd is the plugin's own install directory, and because that directory is itself a git repo, `git rev-parse --show-toplevel` doesn't fail — it silently resolves to the plugin, not the user's workspace, so no ambient-cwd approach can distinguish the two cases.

## Decision

Any `[[panes]]` script that needs to know where the user is working must resolve it from the calling pane, never from `$PWD` or ambient git: `herdr pane get "$HERDR_ACTIVE_PANE_ID" | jq -r .result.pane.cwd`. In `new-agent-popup.zsh`, the fix is a `cd` to that resolved cwd placed immediately after `source config.zsh` and before `wt_mode="$1"` — at the very top of the script, before any worktree-mode discovery or `project_dir` computation runs — rather than patching only the later `project_dir` line. Placing the `cd` this early means every subsequent `$PWD` read, present and future, is automatically correct without new logic having to remember to resolve cwd itself. Tests must also explicitly set/clear `HERDR_ACTIVE_PANE_ID` for isolation, not rely on `cd`-ing into a non-repo temp dir alone.

## Consequences

- New tabs opened from the popup land in the user's actual workspace, in both worktree mode and normal mode, instead of sometimes silently opening inside the plugin's own install directory.
- Popup cwd is documented as unreliable (see CLAUDE.md, "Popup cwd"); future pane scripts must resolve project context from Herdr's session/pane data, never trust process cwd.
- Logic added to `new-agent-popup.zsh` after the early `cd` does not need its own cwd-resolution step, which lowers the chance of reintroducing this bug in that file — but a new pane script that doesn't adopt the same top-of-script `cd` pattern is still exposed and must independently query `HERDR_ACTIVE_PANE_ID`.
- Tests for pane scripts need `HERDR_ACTIVE_PANE_ID` explicitly isolated, not just a `cd` into a temp directory, or a developer's real active pane can leak into a test run.

## Evidence

- **Plugin install dir is itself a git repo, so `git rev-parse --show-toplevel` fails silently in the wrong direction** — a popup pane's cwd is Herdr's plugin install path (`~/.config/herdr/plugins/github/q.workbench-<hash>`), and because that path is itself a checkout, toplevel resolution doesn't error, it just resolves to the plugin instead of the user's workspace; the only reliable source is `herdr pane get "$HERDR_ACTIVE_PANE_ID" | jq -r .result.pane.cwd`. The test suite hit the same class of leak: `tests/new-agent-popup.test.zsh` only `cd`'d into a non-repo temp dir and needed `HERDR_ACTIVE_PANE_ID` explicitly cleared too, or a developer's real pane bled into the test.
  Session `6adc4311-021c-4894-80b7-57dac7754826`, entry `662cfc51-f194-45ad-a301-79e9b61a9908`, 2026-07-22.
- **A line-level fix on `project_dir` was insufficient because worktree discovery reads `$PWD` earlier** — `new-agent-popup.zsh` reads `$PWD` in two places; replacing only the later `project_dir=$(git rev-parse --show-toplevel …)` assignment left worktree-mode branch/worktree discovery (`git rev-parse --is-inside-work-tree`, `git worktree list`, `git worktree add`), which runs during menu collection, still reading the wrong directory. The fix instead `cd`s to the resolved pane cwd immediately after `source config.zsh`, before `wt_mode="$1"` — at the top of the script, before any `$PWD` read happens.
  Session `6adc4311-021c-4894-80b7-57dac7754826`, entry `0d905ec6-0f2a-4caf-8c69-a627c0a6b393`, 2026-07-22.
