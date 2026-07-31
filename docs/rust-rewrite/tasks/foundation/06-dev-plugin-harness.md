# FOUNDATION-06: Dev plugin harness

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
>
> **Depends on**: foundation/01
> **Blocks**: cutover/01
> **Status**: todo

## Goal

A second, linkable Herdr plugin that points at the Rust binary, so every flow can be
driven for real during development without touching the installed `q.workbench`.

## Files to create / modify

- `dev/herdr-plugin.toml` (new) — manifest for plugin id `q.workbench-dev`
- `dev/run.zsh` (new) — shim that execs the sibling binary

## Implementation notes

### Why this exists

Cutover is a single big-bang commit, so the installed plugin keeps running zsh until
the very end. Without a second plugin there is no way to exercise a popup or an
in-pane launcher for real, and several behaviours here — TTY handling, menu centering,
restart-in-place — only show their problems in a live pane.

The installed plugin lives at `~/.config/herdr/plugins/github/q.workbench-<hash>/`.
**Do not modify it.** This harness is entirely separate.

### Manifest

Mirror the real manifest's six actions and five panes, with:

- `id = "q.workbench-dev"`, and titles prefixed so they are distinguishable in Herdr's
  action list (for example `[dev] New agent`).
- `min_herdr_version` and `platforms` matching the real manifest.

**The two entry kinds are wired differently, and this matters.** Five of the six
actions exist only to open a popup pane; their `command` must stay
`["herdr", "plugin", "pane", "open", "--plugin", "q.workbench-dev", "--entrypoint",
"<pane id>", "--placement", "popup", "--width", …, "--height", …]`, with the same
width and height values as the real manifest. Pointing those actions straight at the
shim would run the binary with no popup at all, and every popup, TTY and menu-centering
check in the later tasks would be verifying the wrong thing.

Only the five `[[panes]]` entries and the `dashboard` action point at the shim, e.g.
`command = ["./run.zsh", "agent", "popup"]` and
`command = ["./run.zsh", "agent", "popup", "--worktree"]`.

Note the `--plugin` value must be `q.workbench-dev`, not `q.workbench` — copying the
real manifest without changing it would open the *installed* plugin's zsh popups.

Herdr resolves a relative `command[0]` against the plugin root and starts the process
with that root as its cwd — verified for both `[[actions]]` and `[[panes]]` entries.
Using the shim rather than `../bin/workbench` avoids depending on a parent-relative
path, which was not tested.

### Shim

```zsh
#!/usr/bin/env zsh
exec "${0:A:h:h}/bin/workbench" "$@"
```

`${0:A:h:h}` resolves the script's own directory and then its parent, so the shim
finds `bin/workbench` in the repo root regardless of cwd. Mark it executable and keep
it committed as executable (`git update-index --chmod=+x` if needed).

### Usage

```zsh
herdr plugin link dev/          # register
herdr plugin action list        # confirm the [dev] entries appear
herdr plugin unlink q.workbench-dev
```

Document this in the task's completion note so later tasks can use it without
rediscovering it.

## Acceptance criteria

- [ ] `dev/herdr-plugin.toml` declares `q.workbench-dev` with all six actions and all
      five panes, titles visibly marked as dev.
- [ ] The five popup actions keep the `herdr plugin pane open` form, with
      `--plugin q.workbench-dev` and the real manifest's width and height values.
- [ ] The five panes and the `dashboard` action point at `./run.zsh`.
- [ ] `dev/run.zsh` is executable and execs `bin/workbench` with all arguments
      forwarded.
- [ ] `herdr plugin link dev/` succeeds and the dev actions appear in
      `herdr plugin action list`.
- [ ] Invoking a dev popup action opens an actual popup pane that reaches the Rust
      binary — confirmed by a stub subcommand exiting with its `unimplemented:` message.
- [ ] The installed `q.workbench` is untouched and still enabled.

## Verification

- [ ] `herdr plugin link dev/` then `herdr plugin list` shows both plugins enabled
- [ ] `herdr plugin action list` lists the `q.workbench-dev` actions with the expected
      relative commands
- [ ] Invoke one dev **popup** action and confirm a popup pane actually opens and the
      binary ran inside it (check `herdr plugin log list` or the stub's exit status)
- [ ] Invoke the dev `dashboard` action and confirm it runs the shim directly, with no
      popup
- [ ] `herdr plugin unlink q.workbench-dev` removes it cleanly, leaving `q.workbench`
      enabled

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Modifies the installed plugin, or the shim fails to find the binary | Links but some entrypoints are missing | All entrypoints present, shim resolves correctly, installed plugin untouched |
| Test coverage | ×2 | Never linked | Linked but no action invoked | Link, list, invoke, and unlink all exercised |
| Interface & readability | ×1 | Dev titles indistinguishable from the real ones | Marked inconsistently | Every dev entry clearly marked |
| Assumptions & docs | ×1 | Usage undocumented | Commands listed without context | Link/unlink workflow written down for later tasks |

## Out of scope

- Deciding whether `dev/` ships in the released plugin — settled during cutover.
- Any real flow behaviour; the stubs are enough to prove the wiring.
