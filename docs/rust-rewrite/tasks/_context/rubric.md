# Shared eval rubric

> Every task carries its own `## Eval rubric` table with task-specific anchors. The
> scale, weights, threshold and veto below are shared. A task's table may sharpen the
> anchor wording; it must not change the weights or the threshold.

## Scale

Each dimension is scored 0–5.

- **0–1** — fails the dimension outright.
- **2–3** — below the bar; works in the obvious case but drifts elsewhere.
- **4–5** — passes.

## Dimensions and weights

| Dimension | Weight | What it measures here |
|---|---|---|
| Correctness | ×3 | Does it match the zsh behaviour it replaces, including the edge cases and ordering traps recorded in the parity contract? |
| Test coverage | ×2 | Are the failure paths and cancellation paths covered, not just the happy path? |
| Interface & readability | ×1 | Clear types, no smuggled I/O in pure functions, composable module boundaries, comments that explain why |
| Assumptions & docs | ×1 | Assumptions stated, unverified claims flagged, non-obvious decisions explained |

Weighted average = Σ(score × weight) ÷ Σ(weight), on the same 0–5 scale.

## Pass threshold

A task passes when the weighted average is **> 4.0** and no veto fires.

**Hard-fail veto**: `Correctness < 4` is an automatic veto regardless of the average.
This project is a port with a live reference implementation — a correctness gap is
never offset by good structure.

## Notes for judges

- "Correctness" for this plan means **parity**. A deviation is only correct if it is
  one of the six sanctioned deviations listed in the parity contract.
- A task that silently drops a comment explaining an ordering trap or a TTY quirk
  should lose points on *Assumptions & docs*, not be waved through.
- Cancellation paths matter as much as success paths: a flow that leaves a half-built
  tab behind is a correctness failure, not a polish issue.
