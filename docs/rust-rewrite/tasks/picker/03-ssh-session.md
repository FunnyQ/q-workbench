# PICKER-03: `workbench ssh session`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/02, registry/05
> **Blocks**: picker/05
> **Status**: done

## Goal

Own one SSH connection inside a pane and close that pane's dedicated tab when the
connection ends, however it ends.

## Files to create / modify

- `src/flows/ssh.rs` (new) — the session wrapper
- `src/main.rs` (modify) — wire `ssh session <target> <tab_id>`

## Implementation notes

### What it does

Runs *inside* the pane the SSH picker created:

1. arrange for the tab to be closed on normal exit and on HUP, INT and TERM
2. run `ssh <target>` as a child, inheriting the terminal
3. on exit status zero, stamp the registry's `use` and append a
   `: <epoch>:0;ssh <target>` line to `$HOME/.zsh_history`
4. exit with ssh's own status

### Why this one does not `exec`

The agent launcher ends in `exec` because it must be replaced by the harness. This one
must **not**: it has cleanup to perform after ssh finishes. That asymmetry is
deliberate — note it in the code so the two are not later unified.

### Signal handling — the full sequence

Closing the tab needs a socket round-trip, which is not safe from a signal handler. But
"set a flag and wait" is not enough on its own: after the parent catches the signal,
`ssh` is still running and the parent is still blocked in `wait`, so it would never
reach the close. Specify the whole path:

1. Install handlers for HUP, INT and TERM that do the minimum async-signal-safe thing:
   record which signal arrived and write one byte to a self-pipe. Nothing else.
2. `wait` on the child is interrupted by the signal and returns `EINTR`. **Do not
   blindly retry** — check the flag first.
3. When the flag is set, **forward the same signal to the child**, then resume waiting
   with a bounded retry so a stuck `ssh` cannot hang the process forever. If the child
   is still alive after the timeout, send `SIGKILL` and reap.
4. Once the child is reaped, close the tab from the main path.
5. Exit with `128 + signum` for the signal case, matching shell convention; otherwise
   exit with ssh's own status.

When `wait` returns `EINTR` and the flag is **not** set — a stray signal — retry the
wait as normal.

Do not send the signal to the whole process group: `ssh` is the only child, and
signalling the group risks hitting the pane's shell.

The tab must close in all cases: clean exit, non-zero exit, and each of the three
signals. Missing one leaves an orphaned tab, which is the failure users notice.

### History append

The line format is the zsh extended-history form the registry's seeding also parses:
`: <unix epoch>:0;ssh <target>`. Append with a trailing newline. A failure to append
must not change the exit status — the connection already succeeded.

## Acceptance criteria

- [x] `ssh session <target> <tab_id>` runs `ssh <target>` with the terminal inherited.
- [x] The tab is closed on clean exit, on non-zero exit, and on HUP, INT and TERM.
- [x] Signal handlers do only async-signal-safe work; the socket call happens on the
      main path after the child is reaped.
- [x] On a caught signal the same signal is forwarded to the child, followed by a
      bounded wait and then `SIGKILL` if it is still alive.
- [x] An `EINTR` from `wait` with no flag set retries the wait rather than treating it
      as termination.
- [x] A zero exit stamps the registry's `use` and appends the history line; a non-zero
      exit does neither.
- [x] Exit status is ssh's own status normally, and `128 + signum` after a caught
      signal.
- [x] A failed history append does not change the exit status.

## Verification

- [x] `cargo test` — the history line's exact text for a given target and timestamp
- [x] `cargo test` — with `FakeClient`, a zero exit issues `tab.close` and the
      registry stamp; a non-zero exit issues only `tab.close`
- [x] Manual: run against a host that accepts the connection, disconnect normally, and
      confirm the tab closed and `last_used_at` was stamped
- [x] Manual: run against a host that refuses, and confirm the tab still closes and no
      stamp was written
- [x] `cargo test` — an integration test that substitutes a long-running stand-in for
      `ssh` (a `sleep`-like child), sends **each** of HUP, INT and TERM in turn, and
      asserts for each: the child was signalled, it was reaped, `tab.close` was issued,
      and the exit status is `128 + signum`
- [x] `cargo test` — a stand-in child that ignores the forwarded signal is `SIGKILL`ed
      after the bounded wait, and the tab still closes
- [x] Manual: send SIGTERM to a real session process and confirm the tab closes
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | An orphaned tab survives one of the exit paths | All paths close the tab but the stamp or history line is wrong on a failed connection | Every exit path closes the tab; stamping and history are conditional on success |
| Test coverage | ×2 | No signal test | Clean exit and one signal | Clean exit, failed exit, all three signals, and the ignore-then-KILL case |
| Interface & readability | ×1 | Socket call inside the signal handler | Deferred but the mechanism is convoluted | Minimal handler, self-pipe, forward-then-reap-then-close on the main path, clearly commented |
| Assumptions & docs | ×1 | No note on why this does not `exec` | Mentioned briefly | The `exec` asymmetry and the signal-handler constraint both explained |

## Out of scope

- Picking the host and creating the tab — a separate task creates both and passes the
  tab id in.
