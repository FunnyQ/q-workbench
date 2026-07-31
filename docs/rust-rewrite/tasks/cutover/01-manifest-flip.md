# CUTOVER-01: Flip the manifest and delete the zsh implementation

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/04, foundation/06, polish/01, polish/05, polish/06
> **Blocks**: cutover/02
> **Status**: todo

## Goal

One revertible commit that points every manifest entry at the Rust binary and removes
the zsh implementation and its tests.

## Files to create / modify

- `herdr-plugin.toml` (modify) — all six actions and all five panes
- `scripts/*.zsh` (delete) — the 14 implementation scripts, keeping `build.zsh`
- `tests/*.test.zsh` (delete) — the 9 zsh tests
- `config.example.zsh` (delete) — replaced by `config.example.toml`
- `dev/` (delete) — the whole dev-plugin harness, including its zsh shim

## Implementation notes

### Order of work

Do the verification **before** the deletion, in this order:

1. Rebuild: `zsh scripts/build.zsh`.
2. Run the full old zsh suite one last time and confirm it still passes — it is the
   last moment it can be run.
3. Drive every flow once through the linked dev plugin.
4. Only then edit the manifest and delete.

### Manifest

The six actions currently split into two shapes: `dashboard` runs a script directly,
the other five open a plugin pane via the `herdr` CLI. Both shapes change.

- The five popup-opening actions keep opening panes. Their `command` still invokes
  `herdr plugin pane open` — only the **pane** entries they target change. This is the
  one scoped exemption from the socket-only rule, and it is deliberate: that line is a
  Herdr configuration value that Herdr itself runs, not code this plugin executes.
  Proxying it through the binary would require the binary to learn its own plugin id,
  which differs between the installed and linked plugins and has no verified source.
  **Do not "finish the job" by changing these five lines.**
- `dashboard` becomes `command = ["./bin/workbench", "dashboard"]`.
- Each of the five `[[panes]]` entries becomes `["./bin/workbench", …]` with the
  matching subcommand, for example `["./bin/workbench", "agent", "popup"]` and
  `["./bin/workbench", "agent", "popup", "--worktree"]`.

A relative `command[0]` is verified to work for both entry kinds; the process starts
with the plugin root as its cwd.

Keep every `id`, `title`, `contexts`, `placement`, `--width` and `--height` value
exactly as it is. The manifest's `version` is bumped by the release flow, not here.

### Deletion

Use `trash`, not `rm`. Delete the 14 implementation scripts, the 9 tests,
`config.example.zsh`, and the whole `dev/` directory. Keep `scripts/build.zsh` — it is
the new build path, not part of the old implementation.

`dev/` goes because its purpose ends here: it existed to drive Rust flows while the
installed plugin still ran zsh. Afterwards, linking the repo itself does the same job.
Its `run.zsh` shim would also leave a second zsh script in a repo whose stated goal is
exactly one.

**Unlink it before deleting**: `herdr plugin unlink q.workbench-dev`. A linked plugin
whose directory has vanished is a confusing state to leave Herdr in.

Deleting the tests in the same commit is deliberate: they mock a `herdr` binary that
the Rust version never invokes, so they cannot be adapted, and their coverage has been
reproduced in `cargo test` by the earlier tasks.

### One commit

The manifest flip and the deletion must land together, so `git revert` of that single
commit restores a working plugin. Use `chronicle:commit` with the `simple` argument to
force one commit rather than an atomic split.

### If a polish item was cut

The three behavioural improvements in the `polish` bucket are individually cuttable at
Q's discretion, but this task depends on all of them. A cut item is recorded, not
deleted: its task file gets `Status: done` plus a line in its Goal saying it was cut and
why, and the omission is listed under **Known gaps** in `tasks/README.md`. Confirm that
record exists before flipping, so nobody later mistakes a deliberate cut for an
oversight.

### Install-path check

The installed plugin at `~/.config/herdr/plugins/github/q.workbench-<hash>/` is a git
clone. After the commit is pushed, updating that clone must bring `bin/workbench` with
it. Confirm the committed binary is present and executable in a fresh clone — a file
committed without the executable bit will fail at runtime with a confusing error.

## Acceptance criteria

- [ ] The old zsh suite passed on its final run, and the result is recorded.
- [ ] Every flow was driven once through the dev plugin before deletion.
- [ ] The `dashboard` action and all five `[[panes]]` entries point at
      `./bin/workbench` with the matching subcommand.
- [ ] The five popup-opening actions still invoke `herdr plugin pane open`, unchanged
      apart from nothing — their `--plugin`, `--entrypoint`, `--width` and `--height`
      values stay exactly as they are.
- [ ] Every `id`, `title`, `contexts` and `placement` value is unchanged.
- [ ] The 14 implementation scripts, the 9 tests, `config.example.zsh` and the `dev/`
      directory are gone; `scripts/build.zsh` is the only remaining zsh script.
- [ ] `q.workbench-dev` was unlinked before `dev/` was deleted.
- [ ] `bin/workbench` is committed with the executable bit set.
- [ ] Manifest flip and deletion are a single commit. Documentation is deliberately
      **not** in it — a separate task lands that in the next commit.
- [ ] A fresh clone of the repo yields a runnable `bin/workbench`.

## Verification

- [ ] `for t in tests/*.test.zsh; do zsh "$t" || break; done` — final run, all pass
- [ ] `zsh scripts/build.zsh && cargo test && cargo clippy -- -D warnings`
- [ ] Drive all six flows through the dev plugin: new-agent, new-worktree-agent,
      project, ssh, restart-agent, dashboard
- [ ] `herdr plugin action list` shows the `q.workbench` entries with the new commands
      after the installed clone is updated
- [ ] `rg --files -g '*.zsh'` lists exactly `scripts/build.zsh`
- [ ] `herdr plugin list` no longer shows `q.workbench-dev`
- [ ] `git ls-files --stage bin/workbench` reports mode `100755`
- [ ] Clone the repo to a temp directory and run `./bin/workbench --help` from it
- [ ] Invoke each of the six real actions once from Herdr's action list

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | An action is broken after the flip, the binary lacks the executable bit, or a popup action was rewritten away from `plugin pane open` | The right entries flipped but a width, placement or id drifted | Panes and dashboard flipped, popup actions untouched, everything else byte-identical, fresh clone runs |
| Test coverage | ×2 | Deleted before verifying | Some flows driven | Final zsh run recorded, all six flows driven, fresh-clone check done |
| Interface & readability | ×1 | Deletion and flip split across commits | One commit but with unrelated changes mixed in | One commit containing exactly the flip and the deletion |
| Assumptions & docs | ×1 | No record of the final zsh run | Mentioned without results | Final run recorded; rationale for deleting the tests written into the commit body |

## Out of scope

- Documentation updates — the next task owns those.
- Cutting a release. That happens after the docs land.
