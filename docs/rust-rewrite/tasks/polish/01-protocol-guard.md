# POLISH-01: Protocol guard

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/05
> **Blocks**: cutover/01
> **Status**: todo

## Goal

Fail loudly and once with a useful message when the running Herdr speaks a different
protocol than this binary was built against, instead of producing a cascade of silent
call failures.

## Files to create / modify

- `src/herdr/mod.rs` (modify) — expose the expected protocol and the check
- `src/main.rs` (modify) — run the check before any flow that talks to Herdr

## Implementation notes

### The cost being mitigated

Talking to the socket directly couples this plugin to protocol 17. Nothing in the
manifest expresses that — `min_herdr_version` is a version string, not a protocol
number, and a future Herdr could keep the version scheme while changing the wire
shapes. Without a guard, a protocol bump shows up as a pile of deserialisation errors
in unrelated places.

### Behaviour

- Define the expected protocol as one constant, currently `17`.
- Before running any flow that uses the socket, `ping`. Compare `protocol`.
- **On match, do nothing.** No output, no notification, no log line. This is the common
  case and it runs on every single invocation.
- On mismatch, send one `notification.show` saying that Herdr was upgraded and this
  plugin needs rebuilding, naming both protocol numbers, and exit non-zero.
- If `ping` itself fails — socket missing or unreachable — that is a different error;
  report it as such rather than as a protocol mismatch.

### Cost

`ping` costs ~2.1 ms. Subcommands that make no Herdr call at all must skip it — most
importantly `project source`, which runs once per keystroke. Gate the check on the
subcommand rather than running it unconditionally in `main`.

### Choosing between failing and degrading

The research considered falling back to the `herdr` CLI on mismatch. Do not: the CLI
is the thing being removed, keeping a second transport alive defeats the point, and a
protocol change could break the CLI's own output shape too. Fail with a clear message.

## Acceptance criteria

- [ ] The expected protocol number is a single named constant.
- [ ] A matching protocol produces no output of any kind.
- [ ] A mismatch produces exactly one notification, naming both protocol numbers, and a
      non-zero exit.
- [ ] A `ping` failure is reported as a connection problem, not a protocol mismatch.
- [ ] Subcommands that make no Herdr call do not ping. The complete list:
      `project source`, `project scan`, `project rescan`, `project update`,
      `project use`, `project edit`, `ssh sync`, `ssh list`, `ssh get`, `ssh use`,
      `ssh remove`, **`ssh edit`**, and `config migrate`.
      `ssh edit` only writes files and calls registry operations; pinging would make it
      fail outside Herdr for no functional reason.

## Verification

- [ ] `cargo test` — with `FakeClient`, a matching protocol records only the `ping`
      call and nothing else
- [ ] `cargo test` — a mismatching protocol records exactly one `notification.show`
      whose body contains both numbers
- [ ] `cargo test` — a failing `ping` produces a distinct error message
- [ ] `cargo test` — every subcommand on the no-ping list above issues no `ping`,
      asserted one by one rather than as a sample
- [ ] Manual: run `project source` and confirm no Herdr call is made
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Notifies on success, or pings on the per-keystroke path | Guard works but a connection failure is reported as a mismatch | Silent on match, one clear notification on mismatch, distinct connection error, no ping where unneeded |
| Test coverage | ×2 | No guard tests | Mismatch tested only | Match, mismatch, ping failure, and the no-ping subcommand set |
| Interface & readability | ×1 | Protocol number repeated in several places | One constant but the gate is scattered | One constant, one gate, applied at a single dispatch point |
| Assumptions & docs | ×1 | No note on the coupling being accepted | Mentioned briefly | Explains the coupling, why failing beats falling back, and why some subcommands skip the check |

## Out of scope

- Supporting more than one protocol version.
- Any CLI fallback transport.
