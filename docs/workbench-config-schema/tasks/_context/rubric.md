# Eval rubric — shared scale and dimensions

> Every task carries its own `## Eval rubric` with a threshold line and a weighted table. This file defines the scale and what each dimension looks at, so per-task tables only need task-specific anchors.

## Scoring scale

Each dimension scores 0–5.

- **0–1 (fail)** — the work does not do what the task asked, or it breaks something that worked.
- **2–3 (below bar)** — the happy path works, but edge cases, failure paths, or stated constraints drift.
- **4–5 (pass)** — fully matches the task, edge cases handled, constraints honoured.

## Generic dimensions

| Dimension | What it looks at |
|---|---|
| **Correctness** | Does the code do exactly what the task specified? Are the stated invariants — argv byte-equality, socket-call order, error-before-side-effect — actually held? |
| **Test coverage** | Are there tests for the failure paths, not just the happy path? For this plan specifically: does a rejection have a test asserting the error names the offending value? |
| **Interface & readability** | Are the types clear, is the function shape sane, does the code read like the surrounding module? Does it avoid an indirection used once? |
| **Assumptions & docs** | Are non-obvious decisions flagged in a comment that explains *why*? Are magic numbers named or sourced? Is anything the executor had to guess called out? |

## Scoring & pass line

Weighted average = Σ(score × weight) ÷ Σ(weight), on the same 0–5 scale.

- **Default pass line**: weighted average **> 4.0**.
- **Default hard-fail veto**: **Correctness < 4** vetoes the task regardless of the average.

The closing review task scores different axes — integration, goal fit, consistency, regressions — on this same scale and pass line.

## Anchors specific to this plan

These recur across tasks. Score them under `Correctness`:

- **Parity is byte-equality, not similarity.** "The argv looks right" is a 2. A test comparing against the exact expected `Vec<String>` is a 4.
- **Validation must fire before any socket call.** A rule enforced at launch time rather than at `Config::load()` is a Correctness failure even if the user-visible message is fine.
- **Nerd Font glyphs are written as `\u{...}` escapes in Rust.** A pasted glyph is a Correctness failure — it silently corrupts.
- **No new dependency.** `toml`, `serde`, and `anyhow` are already present; adding anything else without a named failure it prevents is a Correctness failure.
