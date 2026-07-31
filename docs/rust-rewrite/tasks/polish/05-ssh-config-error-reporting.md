# POLISH-05: Error reporting for the SSH and config subcommands

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: polish/04, foundation/04, registry/05, picker/04
> **Blocks**: cutover/01
> **Status**: todo

## Goal

Route the SSH, config and diagnostic subcommands through the stderr channel with the
exact text the parity contract specifies.

## Files to create / modify

- `src/registry/ssh.rs`, `src/config.rs`, `src/flows/ssh.rs` (modify) — return errors
  instead of printing them
- `src/main.rs` (modify) — extend the channel match with these subcommands

## Scope

The remaining stderr subcommands:

`ssh sync`, `ssh list`, `ssh get`, `ssh use`, `ssh remove`, `ssh edit`,
`config migrate`, `herdr ping`.

The stderr channel and the project side already exist. This task extends the channel
match and routes these through it; it does not change the mechanism.

## Implementation notes

### Channel selection

`main` already picks the channel from the subcommand by an explicit match. Extend that
match; do not add a heuristic or a second mechanism.

### Message text

The parity contract's stderr table gives the exact text and exit code for every message
the zsh version had. Reproduce those verbatim, including the ones written to **stdout**
on success (`project-registry: edited <path>` and its two siblings).

For fatal paths the zsh version had no message for, the contract specifies the format
`<subcommand path>: <chained cause>` on stderr, exit 1. Use the `clap` subcommand path
as the prefix so the source is obvious — `ssh sync: …`, `config migrate: …`.

The old `Usage: …` lines disappear: `clap` produces its own usage output for an unknown
or malformed subcommand.

### `herdr ping` is a diagnostic

It is typed at a terminal to check the socket, so a failure there must print the
concrete reason — an unreachable socket, a missing `HERDR_SOCKET_PATH`. It never
notifies: a notification about a broken socket would itself need the socket.

### Cancellation

`ssh edit` is the one interactive subcommand here. Cancelling any of its prompts writes
nothing and exits 0 — unlike the project registry's edit, which reports that nothing
was written. Reproduce each as the contract has it rather than unifying them; the zsh
version differs between the two and this task is not the place to change that.

## Acceptance criteria

- [ ] The channel match is extended, not duplicated or replaced by a heuristic.
- [ ] Every `ssh edit` message in the parity contract's stderr table is reproduced
      verbatim with its exit code, including the stdout success line.
- [ ] Fatal paths with no contract message use `<subcommand path>: <chained cause>` on
      stderr, exit 1 — this covers `ssh sync|list|get|use|remove`, `config migrate` and
      `herdr ping`.
- [ ] None of these subcommands ever issues a `notification.show`.
- [ ] `herdr ping` prints a concrete reason when the socket is unreachable.
- [ ] Cancelling an `ssh edit` prompt writes nothing and exits 0.
- [ ] The old `Usage: …` line for the SSH registry is gone.

## Verification

- [ ] `cargo test` — for each `ssh edit` row of the parity contract's stderr table,
      assert the exact bytes on the right stream and the exact exit code
- [ ] `cargo test` — for each of the eight subcommands, assert no `notification.show`
      is issued on any failure path
- [ ] `cargo test` — an unnamed failure in each of `ssh sync`, `config migrate` and
      `herdr ping` produces `<subcommand path>: <cause>` on stderr, exit 1
- [ ] `cargo test` — cancelling an `ssh edit` prompt leaves the SSH config unchanged and
      exits 0
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A contract message differs, or one of these subcommands notifies | Messages right but a stream or exit code differs | Every message, stream and exit code exact |
| Test coverage | ×2 | No message tests | Some rows covered | Every `ssh edit` row, every no-notification assertion, the fallback format on three subcommands, and the cancel path |
| Interface & readability | ×1 | A second channel mechanism appears | The existing match is duplicated rather than extended | One match, extended in place, easy to audit against the contract |
| Assumptions & docs | ×1 | The `ssh edit` cancel difference unexplained | Mentioned once | Explains why `ssh edit` exits 0 on cancel while the project edit reports |

## Out of scope

- Changing what counts as fatal, or unifying the two cancel behaviours.
- Revisiting the notifying and project subcommands, which already route correctly.
