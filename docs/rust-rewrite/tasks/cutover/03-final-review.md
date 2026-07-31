# CUTOVER-03: Final review

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
> - `../_context/rubric.md`
>
> **Depends on**: cutover/02
> **Status**: done
> **Final review**: true

## Goal

Judge the finished rewrite as one deliverable: does it integrate, does it hold
together, did anything regress, and was the original goal actually met.

## Files to create / modify

- `docs/rust-rewrite/FINAL-REVIEW.md` (new) — the evidence record, structure below
- Anything a finding turns out to require. A fix is a change to whichever file is wrong.

### The evidence record

"Recorded" has to mean a file, or a later session cannot tell what was checked. Write
`docs/rust-rewrite/FINAL-REVIEW.md` with exactly these sections:

1. **Actions** — one row per action: name, pass or fail, and one line on what was
   observed.
2. **Parity walk** — one row per **id** in the parity contract's clause index: id,
   verdict (`holds` / `deviation` / `regressed`), and a note. The index exists so this
   is countable: the row count must equal the index's row count, and no id may be
   missing. Do not invent your own clause boundaries.
3. **Measurements** — release binary size, per-invocation startup, and the
   `project source` median from its 50-run measurement, each with the figure the plan
   predicted alongside.
4. **Searches** — the three greps and their output.
5. **Cuts and known gaps** — every cut polish item, and every finding deferred rather
   than fixed, each with a reason.

A verdict of `regressed` anywhere means the review has not passed.

## Implementation notes

This is the holistic gate. Per-task rubrics already scored each piece; do not re-score
them. Look at what only becomes visible once everything is assembled.

### Integration

Walk each of the six real actions end to end from Herdr's action list, on the
installed plugin, not the dev harness:

1. new-agent — a 3-pane layout, correct labels, correct harness
2. new-worktree-agent — a fresh worktree, all three panes born in it
3. project — an existing project focuses its workspace; a new one builds an agent tab
   on enter and stays plain on alt-enter
4. ssh — connects, and the tab closes itself on disconnect
5. restart-agent — the agent relaunches, the side panes survive, the TTY is clean
   afterwards
6. dashboard — Claude starts with the prompt already processing

Then check the seams between them: the project picker injecting the launcher, the
restart flow reusing the pane label, the pickers' bindings reloading correctly.

### Consistency

- Is there exactly one way to build a pane command string, and does everything use it?
- Do the two entry points genuinely share the decision module, or has special-casing
  crept back in?
- Are the defaults defined in exactly one place, with no `:-`-style fallback repeated
  at a call site?
- Do error notifications have a consistent title and body shape?
- Is the glyph table in the parity contract still accurate, including any glyph added
  by a later task?

### Regressions

The zsh implementation and its tests are gone by now, so the reference is the parity
contract. Go through it clause by clause and confirm each behaviour survives, paying
particular attention to the ones no automated test can cover: menu centering, the
deferred split, TTY recovery after killing codex, and fzf redrawing after an editor
binding.

Also confirm nothing was quietly dropped: every subcommand in the shared context's
surface exists and does something real, and no `unimplemented:` stub survives.

### Did it meet the goal

- One binary; **no process spawn of `herdr` anywhere in `src/`**. Verify by searching
  the source. The five `[[actions]]` in `herdr-plugin.toml` that invoke
  `herdr plugin pane open` are the one scoped exemption and are expected to remain —
  they are a Herdr configuration value Herdr itself runs, and proxying them would
  require an unverified plugin-id lookup. Confirm they are still there and still
  correct; do not treat them as a finding.
- No `jq` invocation anywhere.
- The duplication that motivated the rewrite is actually gone.
- Startup cost and binary size are recorded, and the socket path is measurably faster
  than the CLI path it replaced — particularly on the per-keystroke picker source.

### Cuts

If any `polish` task was cut, say so explicitly and record it as a known gap rather
than leaving it implied.

## Acceptance criteria

- [x] All six actions verified on the installed plugin; the three TTY-only behaviours are recorded as K-2.
- [x] Every id in the parity contract's clause index has a verdict; 64 = 64.
- [x] No `herdr` spawn and no `jq` spawn anywhere in `src/`; the five exempt manifest
      actions are present and correct.
- [x] No `unimplemented:` stub remains; every subcommand does real work.
- [x] Defaults live in one place; pane command strings are built one way.
- [x] Startup cost and binary size recorded and compared against the plan's premises.
- [x] No polish task was cut; all six are done, and that is recorded.
- [x] `docs/rust-rewrite/FINAL-REVIEW.md` exists with all five sections filled in, and
      the parity walk lists every clause id with a verdict.
- [x] Findings are either fixed (F-1..F-14) or written down with a reason (K-4..K-13).

## Verification

- [x] All six actions walked; `project`/`ssh`/`config`/`herdr` exercised live against the real socket and registries
- [x] `rg -n '"herdr"|\bjq\b' src/` returns nothing that spawns either — 4 argv literals, 4 comments
- [x] `rg -c '"herdr", "plugin", "pane", "open"' herdr-plugin.toml` reports `5` — plain
      `rg herdr` would also match `min_herdr_version` and is not a useful check
- [x] `rg -n 'unimplemented' src/` returns nothing
- [x] `cargo test && cargo clippy -- -D warnings` clean — 190 passed, no warnings
- [x] Walked id by id into the record's parity table; both counts are 64, `comm` shows
      no id on either side alone
- [x] `project source` median 3.35 ms against the zsh version's 14.6 ms, budget ≤5 ms
- [x] `git revert --no-commit 7731a62` applies clean in a scratch worktree and the restored zsh suite passes

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto. Correctness here means "the assembled deliverable is right", not "each task was right".

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | An action is broken end to end, or a parity clause silently regressed | All actions work but some clause ids are missing from the walk | Every action verified, every clause id given a verdict, deviations recorded |
| Test coverage | ×2 | Review done by reading only | Some actions exercised | All six actions exercised on the installed plugin, plus the searches and the measurements |
| Interface & readability | ×1 | Duplication or a second command-building path survives | Mostly consolidated, one seam left | One decision module, one command builder, one defaults source |
| Assumptions & docs | ×1 | No evidence record written | Record exists but sections are partial | All five sections complete, every parity clause given a verdict, every figure compared against its prediction |

## Out of scope

- Re-scoring individual tasks. Their own rubrics already did that.
- New features. A finding that amounts to a feature request becomes a known gap, not a
  fix.
