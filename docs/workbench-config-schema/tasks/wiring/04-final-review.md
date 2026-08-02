# WIRING-04: Final review

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
> - `../_context/validation-matrix.md`
>
> **Depends on**: launch/02, wiring/02, wiring/03
> **Blocks**: none — closing task
> **Status**: done
> **Final review**: true

## Goal

The whole schema redesign holds together: the pieces compose, the plan's goal is actually met, nothing regressed, and the committed binary matches the source.

## Files to create / modify

- Any file in the tree (modify) — only to fix an integration defect this review finds. Prefer the smallest correction over a redesign.
- `bin/workbench` (modify) — rebuilt from source as the last step.

## Implementation notes

This is a **holistic gate**, not a re-run of the earlier reviews. Every piece was already scored on its own terms. What this task looks for is what no single piece could see: seams, contradictions, drift from the goal, and things that used to work and now do not.

Read the whole diff before running anything.

### What the plan set out to do

Judge the result against these, not against the individual pieces:

1. A user describes a tab's pane arrangement in TOML and the launcher builds it, instead of five hardcoded socket calls.
2. A user adds, removes, or reorders harnesses and model options in TOML, without recompiling.
3. Omitting a choice means the launcher asks for it, so a layout that pins nothing reproduces today's popup exactly.
4. The CCR special case is gone; it is an ordinary option with a command override.
5. Every configuration error surfaces at config load, before the first socket call.

If any of these is only half-true, that is a finding, even when every underlying piece passed its own gate.

### The parity baselines

These are the numbers the whole redesign is measured against. Reassemble them from the *current* code and compare byte for byte.

Six launch argv lines the built-in defaults must still produce:

```
claude --model claude-opus-4-8
claude --model opusplan --effort medium
ccr code
claude --model claude-fable-5
codex
opencode
```

The socket-call sequence the default layout must still produce, in this order:

1. `pane.split` — target = agent pane, `direction: "right"`, `ratio: 0.38`, `cwd`, `env: {"Q_NO_BANNER": "1"}`, `focus: false`
2. `pane.rename` — new pane, `label: "\u{f0968}  Files"`
3. `pane.send_input` — new pane, `text: "yazi ."`, `keys: ["enter"]`
4. `pane.split` — target = files pane, `direction: "down"`, `ratio: 0.9`, `cwd`, `focus: false`, **no `env`**
5. `pane.rename` — new pane, `label: "\u{f489}  term"`

Note the asymmetry in step 4: no `env` key at all, not an empty one. And note that the config values producing `0.38` and `0.9` are `0.62` and `0.1`, because each pane's ratio describes its own share and the loader converts with `1 - ratio`.

### The validation rules

`../_context/validation-matrix.md` carries the complete list: **25 rejection branches and 3 positive tests**. Walk it row by row. For each branch, name the test function that covers it, and report every row with no test. A summary count is not enough — several rules carry independent branches that can each regress alone, which is why the matrix is written per branch rather than per numbered rule.

Two branches are the most likely to have been softened, so check them by hand:

- **The root-pane rule (5b)** is often weakened into "exactly one agent pane somewhere". It is stronger: the *position* is fixed too, because restart-in-place depends on the agent being the pane the others were split off. The injected launcher ends with `exec`, so killing the harness returns that pane to its surviving shell while the side panes live on.
- **The empty-command branches (11c, 11d, 11e)** are easy to skip, because `[]` and `""` deserialize without complaint and look valid. They are the difference between an error at load and an empty argv failing after the tab is already on screen.

### Seams worth probing

- A layout resolved from the flag and a layout resolved from the default must flow through identically — same construction, same ordering.
- The stored `use last` record and the layout it names must agree. A record naming a layout that a later config edit removed has to be dropped, not replayed.
- Reinjection crosses a shell boundary twice: once when the launcher command is typed into the pane, once when the harness argv is built. Every executable path and argument must be quoted separately at both.
- The restart path passes both a layout name and the do-not-build-panes flag. Confirm those stayed orthogonal and that the TTY reset sequence still leads the injected string.
- Confirm no code path builds panes before the menus finish. Splitting earlier resizes the pane the menus are drawing into, and the chosen worktree must determine the cwd of every pane.

### Post-run hand-verification checklist for Q

**This is not a gate on this task.** Do not block, do not wait, do not ask. Autopilot must finish unattended. Copy this list into the completion report so Q has it when he next opens Herdr.

1. Open the New agent popup — harness, model, and usage menus all appear, in that order.
2. The resulting tab is agent / Files / term, with the agent pane **narrower** than the Files column.
3. Open the New personal assistant popup — no menus at all; it opens straight to Opus with a `machine stats` pane beside it.
4. Restart the personal assistant tab — it relaunches with no menus.
5. Restart an agentic-coding tab — `use last` is the first entry in the harness menu.
6. Project picker: plain enter opens a workspace with an injected agent; alt-enter leaves it plain.
7. `workbench dashboard` still finds its workspace and opens the tab.

### Fixing what you find

Apply the smallest correction that resolves each finding. If a finding needs a redesign rather than a fix, do not attempt it — record it plainly in the completion report and leave the code working. A half-applied redesign is worse than a named gap.

Rebuild `bin/workbench` **last**, after every source edit is settled.

## Acceptance criteria

- [x] The five plan goals are each demonstrably met, with the evidence named.
- [x] The six launch argv lines match the baseline byte for byte.
- [x] The default layout's socket-call sequence matches the baseline, including the missing `env` on the second split.
- [x] All 25 rejection branches in the validation matrix are enforced at config load, and each is named alongside the test function covering it.
- [x] No leftover reference to the removed settings, subcommand, or harness constants remains in `src/`.
- [x] No Rust source file contains a literal Nerd Font glyph outside a `\u{...}` escape.
- [x] `bin/workbench` is rebuilt from the final source and its bytes changed.
- [x] Any finding that was not fixed is recorded in the completion report with a reason.

## Verification

- [x] `cargo test` — the whole suite passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `zsh scripts/build.zsh`, then `git status --short bin/workbench` shows it modified. A stale committed binary is the usual cause of code and behaviour disagreeing, because a linked checkout runs that artifact rather than the source.
- [x] `rg -n 'config migrate|claude_extra_args|codex_extra_args|model_args|config\.order|HARNESS_CLAUDE|HARNESS_CODEX|HARNESS_OPENCODE' src/` returns nothing.
- [x] `python3 -c "import pathlib;[print(p,hex(ord(c))) for p in pathlib.Path('src').rglob('*.rs') for c in p.read_text() if 0xE000<=ord(c)<=0xF8FF or 0xF0000<=ord(c)<=0xFFFFD or 0x100000<=ord(c)<=0x10FFFD]"` returns nothing — every Nerd Font glyph in Rust is a `\u{...}` escape, never a literal. The ranges are the three Unicode private-use areas, which is where Nerd Font glyphs live. Do **not** widen this to "any codepoint above U+2FFF": ordinary CJK, arrows, and box-drawing characters in comments and strings are legitimate and would fail the gate for no reason.
- [x] Grep the test suite for every one of the 25 matrix branches and confirm a test exists for each; list any branch that has none.
- [x] Run `git status --short` and quote it. Expect `bin/workbench`, plus any source file this review had to correct, plus at most this task file. Anything unexpected is a scope violation worth explaining in the report.

## Eval rubric

> Scale 0–5 on each dimension; weighted average > 4.0 to pass; Integration < 4 is an automatic veto. This gate scores the whole deliverable, not the individual pieces.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Integration | ×3 | The pieces do not compose — a seam is broken, or the flag path and default path behave differently | Composes, but a seam is unexamined: quoting, ordering, or the stored-record-to-layout agreement was not probed | Every seam probed and sound; flag and default paths identical; both shell boundaries quoted |
| Meets the plan goal | ×3 | A stated goal is unmet and unreported | Goals met on the surface; the pinning-nothing layout does not truly reproduce today's popup | All five goals demonstrably met, each with named evidence |
| Consistency | ×2 | Documents contradict the code, or the committed binary is stale | Mostly consistent; one document or the binary lags | Source, documents, example config, and committed binary all agree |
| No regressions | ×2 | A parity baseline drifted and was not caught | Baselines match but a validation rule lost its test | Both baselines byte-identical, every validation rule enforced and tested, full suite green |

## Out of scope

- Redesigning anything a finding exposes as structurally wrong — Deferred. Record it in the completion report; a half-applied redesign at the closing gate is worse than a named gap.
- Committing or tagging — Deferred. Committing is done with the repository's commit skill, and releasing is a separate decision that belongs to Q.
- The deferred workspace-from-a-list feature — Deferred. It was cut from this plan's scope and has no implementation to review.
