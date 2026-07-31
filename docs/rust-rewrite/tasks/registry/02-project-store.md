# REGISTRY-02: Project registry storage and non-interactive operations

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: registry/01
> **Blocks**: registry/03, picker/01, polish/04
> **Status**: todo

## Goal

The registry file itself — schema, atomic write, merge — plus `update` and `use`, the
two operations that need no prompts.

## Files to create / modify

- `src/registry/project.rs` (modify) — the store, the merge, `update`, `use`
- `src/main.rs` (modify) — wire `project update` and `project use`

## Implementation notes

The registry path, schema, source-accumulation rule, and the per-operation guards are
inlined in the parity contract.

### Replacing the jq pipeline

`scripts/project-registry.zsh` merges discovered candidates with the existing registry
inside a jq program. With `serde_json` this becomes ordinary Rust — a
`BTreeMap<String, ProjectEntry>` and a merge function. Take the readability win; do not
transcribe the jq shape.

Use `BTreeMap`, not `HashMap`. Key order must be stable for the parity comparison and
for a clean diff of the registry file.

Keep the merge a **pure function** over `(existing registry, discovered map) ->
registry`, so it can be tested without touching the filesystem.

### Atomic write

Write to a `mktemp` sibling of the destination, pretty-print, then rename over the
target. If anything fails, `trash` the temp file. **Never** truncate the destination
first.

`jq '.'` emits two-space indentation and a trailing newline. `serde_json::to_string_pretty`
uses two spaces; append the newline explicitly, or the parity comparison fails on
whitespace alone.

### The timestamp

`generated_at` is a current UTC timestamp in `%Y-%m-%dT%H:%M:%SZ`. The standard library
has no calendar conversion, so format it with the `time` crate — build a
`format_description` matching that pattern exactly rather than reaching for a
general-purpose RFC 3339 helper, which emits subseconds and an offset the zsh version
does not.

`last_used_at` is a plain unix epoch and needs no crate:
`SystemTime::UNIX_EPOCH.elapsed()`.

Two runs can never produce literally equal files, so make the clock injectable — a
parameter or a small trait, defaulted to the real clock. Tests pin it; the
cross-implementation comparison normalises it.

### Operations

- **`update`** — refresh `sources` from discovery with no prompt, preserving `manual`.
  Requires an existing valid registry.
- **`use`** — stamp `last_used_at` on the given project, creating the entry with
  `sources: ["manual"]` if it is absent. Requires an existing registry and a path that
  canonicalises.

Both print their success line to stdout and their failures to stderr, with the exact
text and exit codes in the parity contract's message table.

## Acceptance criteria

- [ ] The schema matches the parity contract, `version: 1`, keys in stable order.
- [ ] Writes are atomic; a failure leaves the previous registry intact and no temp file
      behind.
- [ ] Output is two-space indented with a trailing newline.
- [ ] The clock is injectable and the timestamp format is `%Y-%m-%dT%H:%M:%SZ` UTC.
- [ ] `sources` accumulate, stay sorted-unique, and preserve `manual`.
- [ ] The merge is a pure function that touches no filesystem.
- [ ] `update` and `use` enforce their guards and emit the exact messages and exit codes
      from the parity contract.

## Verification

- [ ] `cargo test` — merge tests: a newly discovered project, a project that
      disappeared from discovery, and a `manual` source surviving an `update`
- [ ] `cargo test` — atomic write: a simulated serialisation failure leaves the original
      file byte-for-byte unchanged and removes the temp file
- [ ] `cargo test` — timestamp format, with a pinned clock
- [ ] `cargo test` — every guard message and exit code for `update` and `use`
- [ ] **Parity check**: run the zsh `scripts/project-registry.zsh update` and the Rust
      `project update` against the same registry copy and the same fixture sources,
      replace `generated_at` in both with a constant, then `diff`. They must match.
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Output differs after normalisation, or a write is non-atomic | Matches on the simple case but a guard message or `manual` preservation differs | Normalised output identical; every guard and message reproduced |
| Test coverage | ×2 | No merge tests | Merge only | Merge, atomic-write failure, timestamp, every guard, and the parity diff |
| Interface & readability | ×1 | The jq shape transcribed into Rust | Readable but merge takes I/O | Merge is pure over maps; I/O confined to the store |
| Assumptions & docs | ×1 | No note on why the clock is injectable | Mentioned briefly | Explains the timestamp problem and why `jq`'s exact formatting is matched |

## Out of scope

- Discovery itself — it already exists and is consumed here.
- The interactive review and edit flows — the next task.
