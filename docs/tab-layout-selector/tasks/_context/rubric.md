# Eval rubric — shared bar

> Every task carries its own `## Eval rubric` with a threshold line and a weighted table.
> This file pins the scale and the generic dimension definitions so task tables only need
> task-specific anchors.

## Scoring scale (0–5)

- **0–1 (fail)** — the stated outcome is not achieved, or it is achieved by breaking
  something else.
- **2–3 (below bar)** — the happy path works, but edge cases, tests, or naming drift from
  what the task asked for.
- **4–5 (pass)** — fully matches the task, edge cases handled, tests cover the failure
  paths, and a reader can follow the code without the task file.

## Generic dimensions

- **Correctness** — does the code do exactly what the task specifies, including the
  ordering and protocol constraints called out in `shared.md`? Does it leave existing
  behaviour untouched where the task says "no behaviour change"?
- **Test coverage** — are new paths covered by `cargo test`, including cancellation and
  the error branches? Are existing tests still meaningful, not weakened to pass?
- **Interface & readability** — are signatures narrow and typed, is visibility no wider
  than needed, and does the code match the style of the file it lands in?
- **Assumptions & docs** — are non-obvious ordering, protocol, terminal, and quoting
  reasons commented? Are assumptions stated rather than silently baked in?

## Scoring & pass line

Weighted average = Σ(score × weight) ÷ Σ(weight), on the same 0–5 scale.

- Default weights: Correctness ×3, Test coverage ×2, Interface & readability ×1,
  Assumptions & docs ×1.
- Pass threshold: `> 4.0`.
- Hard fail: `Correctness < 4` is an automatic veto regardless of the average.

The closing review task replaces the four dimensions with integration axes (Integration,
Meets the goal, Consistency, No regressions) and keeps the same scale and threshold.
