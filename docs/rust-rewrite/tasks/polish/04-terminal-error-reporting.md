# POLISH-04: The stderr channel and the project subcommands

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: polish/03, registry/02, registry/03, picker/01
> **Blocks**: polish/05
> **Status**: todo

## Goal

Attach the stderr channel to the reporting mechanism, and route the six project
subcommands through it with the exact text the parity contract specifies.

## Files to create / modify

- `src/main.rs` (modify) — select the channel by subcommand
- `src/registry/project.rs`, `src/flows/picker.rs` (modify) — return errors instead of
  printing them

## Scope

The channel-selection mechanism, plus the six project subcommands:

`project scan`, `project rescan`, `project update`, `project use`, `project edit`,
`project source`.

The reporting mechanism, the `Outcome` type and every notifying subcommand already
exist. This task adds the second channel and routes the project side through it. The
SSH and config subcommands follow in the next task; they need the channel to exist
first, and the project side is where most of the message table lives.

## Implementation notes

### Channel selection

`main` picks the channel from the subcommand, not from the error. A notification is
right when the process has no durable place to print; stderr is right when the user is
looking at a terminal. The two lists are fixed and small — encode them as a match on
the parsed subcommand rather than a heuristic.

### Message text

The parity contract's stderr table gives the exact text and exit code for every message
the zsh version had. Reproduce those verbatim, including the ones written to **stdout**
on success (`project-registry: edited <path>` and its two siblings).

For fatal paths the zsh version had no message for, the contract specifies the format
`<subcommand path>: <chained cause>` on stderr, exit 1. Use the `clap` subcommand path
as the prefix so the source is obvious — `ssh sync: …`, `config migrate: …`.

The old `Usage: …` lines disappear: `clap` produces its own usage output for an unknown
or malformed subcommand.

### `project source` is exempt

It never notifies and never grows new output. It runs once per keystroke through the
picker's `change:reload` binding; a notification storm on a transient registry read
would be unusable, and extra stderr would be equally noisy. A failure there stays a
bare non-zero exit, exactly as today.

### Cancellation

The interactive project subcommands — `project scan`, `rescan` and `edit` — treat a
cancelled prompt as a **failure with a specific message**, not as
`Outcome::Cancelled`. That is what the zsh version does, and the messages are in the
contract (`project-registry: edit cancelled; registry not written`, and so on), exit 1.
Do not silently succeed on these. `ssh edit` behaves the same way and is handled in the
next task.

That asymmetry with the popup flows is deliberate: cancelling a popup is a normal
gesture with nothing to report, whereas cancelling a registry edit at a terminal
deserves a line saying nothing was written.

## Acceptance criteria

- [ ] `main` selects the channel from the subcommand, by an explicit match over the two
      fixed lists.
- [ ] Every `project-*` message in the parity contract's stderr table is reproduced
      verbatim with its exit code, including the three stdout success lines.
- [ ] Fatal paths with no contract message use `<subcommand path>: <chained cause>` on
      stderr, exit 1.
- [ ] No project subcommand ever issues a `notification.show`.
- [ ] `project source` neither notifies nor gains new output; a failure is a bare
      non-zero exit.
- [ ] Cancelling `project scan`, `rescan` or `edit` reports its contract message and
      exits 1.
- [ ] The old `Usage: …` line for the project registry is gone.

## Verification

- [ ] `cargo test` — for each `project-*` row of the parity contract's stderr table,
      assert the exact bytes on the right stream and the exact exit code
- [ ] `cargo test` — for each project subcommand, assert no `notification.show` is
      issued on any failure path
- [ ] `cargo test` — an unnamed failure produces `<subcommand path>: <cause>` on stderr
- [ ] `cargo test` — `project source` failure produces no output and a non-zero exit
- [ ] `cargo test` — cancelling each of the three interactive project subcommands
      produces its message and exit 1, and leaves the registry unchanged
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A contract message differs, or a stderr subcommand notifies | Messages right but a stream or exit code differs, or `project source` gains output | Every message, stream and exit code exact; `project source` untouched |
| Test coverage | ×2 | No message tests | Some rows covered | Every table row, every no-notification assertion, the fallback format, and all three cancel paths |
| Interface & readability | ×1 | Channel chosen by a heuristic | Explicit but the lists are duplicated | One match over two named lists, easy to audit against the contract |
| Assumptions & docs | ×1 | The cancel asymmetry unexplained | Mentioned once | Explains why terminal cancellation reports while popup cancellation does not |

## Out of scope

- Changing what counts as fatal.
- Revisiting the notifying subcommands, which already route correctly.
- The SSH and config subcommands — the next task, using the channel built here.
