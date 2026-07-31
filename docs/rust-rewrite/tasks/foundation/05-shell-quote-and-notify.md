# FOUNDATION-05: Shell quoting and the notification helper

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
>
> **Depends on**: foundation/02
> **Blocks**: picker/05, agent/02, agent/03, agent/05, polish/01
> **Status**: todo

## Goal

A correct `shell_quote()` for the command strings sent into panes, and one notify
helper every fatal path can call.

## Files to create / modify

- `src/shell.rs` (new) — `shell_quote()` and a small `build_command()` joiner
- `src/notify.rs` (new) — `notify()` over `notification.show`

## Implementation notes

### Why quoting matters here

`pane.send_input` types text into a pane, and **the pane's interactive shell then
interprets it**. Anything embedded — a path, a branch name, an SSH target, a launch
argument — must be quoted before it goes in. The zsh version got this for free from
`${(q)}`. In Rust it has to be implemented, and it is the one genuinely new failure
surface the rewrite introduces: a quoting bug produces a silently wrong command, not
an error.

The socket removes the *second* layer of quoting that CLI argv needed. It does not
remove this one.

### `shell_quote()`

Single-quote everything and escape embedded single quotes by closing, escaping and
reopening — the standard POSIX-safe form:

```rust
/// Quote `s` so a POSIX shell reads it back as exactly one literal argument.
pub fn shell_quote(s: &str) -> String
```

`don't` becomes `'don'\''t'`. An empty string becomes `''` — not the empty output,
which would drop the argument entirely. Do not special-case "looks safe, skip the
quotes": the readability gain is not worth the class of bug it opens.

`build_command(parts: &[String]) -> String` joins quoted parts with single spaces, and
is what every caller uses instead of formatting a string by hand.

Note one deliberate exception the callers rely on: the restart flow prefixes the
injected command with a literal `stty sane; printf '…';` sequence that is **meant** to
be interpreted by the shell. That prefix is not passed through `shell_quote`; only the
launcher path and its arguments are. Keep the two clearly separated so nobody
"fixes" it later.

### `notify()`

```rust
pub fn notify(client: &dyn HerdrClient, title: &str, body: &str);
```

Calls `notification.show` with `position: "bottom-right"` and `sound: "none"` unless a
caller needs otherwise, and **swallows its own failure** — a notification that cannot
be delivered must never mask the error it was reporting. Log nothing to stdout: these
flows run in popups and panes where stray output corrupts the display.

Provide a companion for the common shape "report this error and exit non-zero" so
call sites stay one line.

## Acceptance criteria

- [ ] `shell_quote` produces a string a POSIX shell reads back as one literal argument
      for: spaces, single quotes, double quotes, `$`, backticks, `\`, `*`, `?`, `~`,
      newlines, and the empty string.
- [ ] `shell_quote("")` returns `''`.
- [ ] `build_command` joins quoted parts with single spaces.
- [ ] `notify` sends `notification.show` and never propagates its own failure.
- [ ] Neither helper writes to stdout or stderr.

## Verification

- [ ] `cargo test` — a table-driven test over every character class listed above
- [ ] `cargo test` — a round-trip test that runs `zsh -c "printf '%s' <quoted>"` and
      asserts the output equals the original string, for each case
- [ ] `cargo test` — `notify` against `FakeClient` records one `notification.show`
      call with the expected params, and a failing fake still returns without panicking
- [ ] Live smoke test in a scratch tab: send a command containing a space, a single
      quote and a `$` through `pane.send_input`, and confirm the pane received it
      literally. Close the tab afterwards.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Embedded single quotes or the empty string break | Handles the common cases, misses newlines or backslashes | Every listed character class round-trips through a real shell |
| Test coverage | ×2 | Assertion on the output string only | Table test, no shell round-trip | Table test plus a real `zsh -c` round-trip plus the live smoke test |
| Interface & readability | ×1 | Callers still format command strings by hand | Helper exists but is bypassed somewhere | `build_command` is the only way callers build pane commands |
| Assumptions & docs | ×1 | No note on the deliberately unquoted restart prefix | Mentioned vaguely | Clearly documented so it is not "fixed" later |

## Out of scope

- Applying the helpers across the codebase — each flow task uses them as it lands, and
  a later task sweeps for consistency.
