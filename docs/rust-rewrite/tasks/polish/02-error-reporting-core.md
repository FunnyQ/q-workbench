# POLISH-02: Error-reporting core and the agent flows

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: agent/02, agent/03, agent/04
> **Blocks**: polish/03
> **Status**: done

## Goal

One reporting mechanism in `main`, plus the agent flows routed through it, so a failure
that used to close a popup silently now says what went wrong.

## Files to create / modify

- `src/main.rs` (modify) — the outcome type and the single reporting path
- `src/flows/*.rs` (modify) — return errors instead of reporting them

## Scope

The mechanism, plus the five agent subcommands that exercise it:

`agent popup`, `agent launch`, `agent inject`, `agent restart`, `agent restart-worker`.

**`agent restart-worker` matters most here.** It runs detached with all three streams
redirected to `/dev/null`, so stderr is physically discarded — a notification is its
*only* way to report anything. It also performs the most failure-prone sequence in the
plugin: resolving a pane, killing a process group, and re-injecting a command. A
failure there today leaves the agent killed and never relaunched, silently. Every one
of its fatal paths must notify.

The agent flows come first because they own the preserved message strings, so the
mechanism gets designed against the hardest case. Two later tasks migrate the remaining
notifying flows and then attach the stderr channel — neither changes the mechanism.

## Implementation notes

### The problem being fixed

The zsh version pipes most Herdr calls to `>/dev/null 2>&1` and exits on failure. Only
five places report: the incomplete-tab cleanup, the two restart failures, the project
picker's two messages (which it writes to a stderr nobody can see), and the dashboard's
missing workspace. Everything else fails invisibly — a popup simply closes, with no way
to tell a cancellation from a failure.

### The outcome type

Three success shapes, plus the error case. Cancellation is not an error, and neither is
"nothing to do here":

```rust
enum Outcome {
    Done,
    Cancelled,
    /// Succeeded with nothing to do, but the user should be told. Exit 0.
    Notice { title: String, body: String },
}
type FlowResult = anyhow::Result<Outcome>;
```

- A user pressing escape at a menu, or fzf exiting non-zero because nothing was picked,
  returns `Ok(Outcome::Cancelled)` — silent, exit 0. Getting this wrong turns every
  escape keypress into a notification, which is worse than the current behaviour.
- **`Notice` exists for one real case**: restart with no agent pane in the tab must
  notify `No agent pane in this tab to restart.` **and exit 0**. That is neither an
  error (which exits non-zero) nor silent. Without this variant the flow would have to
  call `notify` itself, breaking the single-reporting-path rule.

`main` emits the notification for `Notice` exactly as it does for an error, then exits
0.

### One notification per failure

Some flows already notify before returning — most obviously the incomplete-tab cleanup,
whose title is `Agent tab failed` and whose body is `The incomplete tab was closed.`
Left alone that would produce two notifications once a top-level handler exists, and
its body carries no concrete cause.

The contract:

- **A flow never calls `notify` itself.** Cleanup still runs — the tab is still closed
  — but the flow returns the error rather than reporting it.
- **`main` emits exactly one notification.** Its title is the flow's existing title
  where one exists (`Agent tab failed`, `Restart agent`), or a short title naming the
  flow where none does. Later tasks add the remaining titles.
- **The body is the preserved sentence followed by the chained cause**, for example
  `The incomplete tab was closed. pane.split failed: connection refused`. The sentence
  survives verbatim as a prefix; the cause is appended.

Model this so it cannot be got wrong by accident: give the error type an optional title
and an optional preserved-prefix sentence, set at the point the flow already knew them.

`anyhow`'s context chain is what makes the appended cause useful, so this is really two
jobs: route errors to one place, **and** add `.context("…")` at every I/O, parse and
Herdr boundary. A notification reading "error" is no better than silence.

Bodies are one line. Never a backtrace.

Moving where those existing notifications are emitted is expected and is **not** a
parity regression: the strings survive, the emission point moves.

### Delivery cannot itself fail loudly

The notify helper already swallows its own errors. Verify that holds: if the socket is
gone, the process must still exit non-zero rather than panicking while trying to report.

## Acceptance criteria

- [x] `Outcome` distinguishes done from cancelled; each of the five agent subcommands
      returns it.
- [x] `agent restart-worker` notifies on every fatal path — a failed pane resolution,
      a failed kill, and a failed re-injection each produce exactly one notification.
- [x] Restart with no agent pane in the tab returns `Notice`, producing the preserved
      notification and **exit 0** — not an error, not silence.
- [x] No flow module calls `notify` directly; cleanup still runs, reporting moves to
      `main`.
- [x] Every fatal path in a listed subcommand produces **exactly one** notification,
      never two.
- [x] Cancellation produces no notification and exits 0 at every agent menu and at the
      restart confirmation. `agent inject` and `agent restart-worker` have no
      cancellation point and must never return `Cancelled`.
- [x] The preserved strings survive verbatim — titles as titles, sentences as the body
      prefix, with the chained cause appended.
- [x] Bodies are a single line, never a backtrace.
- [x] A failing notification does not panic and does not mask the original error.
- [x] Nothing is written to stdout or stderr by any of the five.

## Verification

- [x] `cargo test` — for each listed subcommand, inject a failure at its first Herdr
      call and assert exactly one `notification.show` with a non-empty, non-generic body
- [x] `cargo test` — for `agent restart-worker`, inject a failure at each of the three
      stages (resolve, kill, re-inject) and assert one notification per stage
- [x] `cargo test` — a tab with no agent pane yields one notification with the preserved
      body and an exit status of 0
- [x] `cargo test` — inject a failure **after** the popup's `tab.create` and assert
      exactly one notification: title `Agent tab failed`, body starting with
      `The incomplete tab was closed.` and continuing with the cause, plus a `tab.close`
- [x] `cargo test` — cancellation at each of the four agent menus, and at the restart
      confirmation, records zero notifications and exits 0. These five are the only
      cancellation points in scope; `agent inject` and `agent restart-worker` have none
      and are asserted to never return `Cancelled`
- [x] `cargo test` — the preserved strings asserted verbatim as title or body prefix
- [x] `cargo test` — a `FakeClient` whose `notification.show` also fails still yields a
      non-zero exit and no panic
- [x] Manual through the linked dev plugin: run `agent restart` from a tab containing no
      agent pane, and confirm one notification reading `No agent pane in this tab to
      restart.` and exit 0. (Do **not** use a failing `git worktree add` for this — that
      is a documented fallback that proceeds without a worktree, not a fatal path.)
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Cancellation notifies, or a preserved string changed | Errors reported but bodies are generic, or one flow double-notifies | Exactly one concrete notification per failure; cancellation silent; strings intact |
| Test coverage | ×2 | No injection tests | One subcommand covered | Every listed subcommand covered for failure and cancellation, plus the failing-notification case |
| Interface & readability | ×1 | Notify calls scattered through the flows | Centralised but flows still exit early in places | One reporting path; flows only return |
| Assumptions & docs | ×1 | No distinction documented between cancel and error | Mentioned once | The three-outcome rule stated where `Outcome` is defined |

## Out of scope

- The picker, SSH and dashboard flows — the next task migrates them onto this mechanism.
- The terminal-facing subcommands and the stderr channel — a later task.
- Changing what counts as fatal. This routes existing failures; it does not make
  previously tolerated conditions fatal.
- Logging to a file.
