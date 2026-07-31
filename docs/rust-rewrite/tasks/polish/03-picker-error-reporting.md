# POLISH-03: Route the picker, SSH and dashboard flows through the reporting core

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: polish/02, picker/02, picker/05, agent/05
> **Blocks**: polish/04
> **Status**: todo

## Goal

The four non-agent notifying subcommands stop reporting for themselves and return
errors through the reporting core, with concrete causes attached.

## Files to create / modify

- `src/flows/picker.rs` (modify) — `project pick`, `ssh pick`
- `src/flows/ssh.rs` (modify) — `ssh session`
- `src/flows/dashboard.rs` (modify) — `dashboard`

## Scope

`project pick`, `ssh pick`, `ssh session`, `dashboard`.

The `Outcome` type, the error type carrying an optional title and preserved-prefix
sentence, and the single reporting path in `main` already exist. This task migrates
these four onto them; it does not change the mechanism.

## Implementation notes

### Per-flow titles and preserved sentences

| Flow | Title | Preserved sentence |
|---|---|---|
| `project pick` | `Project picker` | its two contract messages, used as the whole body |
| `ssh pick` | `SSH picker` | none |
| `ssh session` | `SSH session` | none |
| `dashboard` | `Dashboard Launcher` | `Workspace '<label>' was not found.` — the **whole** body; nothing is appended |

`project pick`'s two messages move from stderr to a notification body, unchanged in
text. That is a fix, not a drift: the picker runs inside a popup pane, so its stderr
was never visible.

Where there is no preserved sentence, the body is the chained cause alone.

Two bodies are complete on their own and take **no** appended cause: the dashboard's
missing-workspace message and the project picker's two messages. Each already names its
concrete cause, and appending a chained context would only repeat it. Appending applies
to the agent popup's `The incomplete tab was closed.`, which describes the cleanup
rather than the failure.

### Cancellation

Only two of these four can be cancelled: `project pick` and `ssh pick`, when fzf exits
non-zero because nothing was picked. Both return `Outcome::Cancelled` — silent, exit 0.

`ssh session` and `dashboard` have no cancellation interaction and must never return
`Cancelled`.

### Context to add

Add `.context("…")` at each Herdr call and each external-process boundary so the body
is specific. The useful boundaries here are: reading the project registry, spawning
fzf, `session.snapshot`, `workspace.create`, `tab.create`, `pane.send_input`,
`workspace.list`, and spawning `ssh`.

### `ssh session` closes its tab first

Its cleanup — closing the tab — must still run before the error propagates, exactly as
the popup's `tab.close` does. A failed connection that leaves the tab open is a worse
outcome than a missing notification.

## Acceptance criteria

- [ ] All four flows return `Result<Outcome>` and call `notify` nowhere.
- [ ] Each carries the title from the table above. `dashboard` and `project pick` use
      their contract message as the **entire** body; nothing is appended to either.
- [ ] `project pick`'s two contract messages appear as notification bodies, unchanged.
- [ ] `project pick` and `ssh pick` return `Cancelled` on an empty fzf exit; the other
      two never return `Cancelled`.
- [ ] `ssh session` still closes its tab on every exit path before the error propagates.
- [ ] Every listed boundary carries a `.context("…")` making the body specific.
- [ ] None of the four writes to stdout or stderr.

## Verification

- [ ] `cargo test` — for each of the four, inject a failure at its first Herdr call and
      assert exactly one `notification.show`, with the expected title and a body naming
      the boundary
- [ ] `cargo test` — `project pick` with a missing registry produces the contract
      message verbatim as the body
- [ ] `cargo test` — `dashboard` with no matching workspace produces a body equal to
      `Workspace '<label>' was not found.` exactly, with nothing appended
- [ ] `cargo test` — a cancelled fzf in each picker records zero notifications and exits 0
- [ ] `cargo test` — `ssh session` failing mid-flow still issues `tab.close`
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A tab is left open, or a cancelled picker notifies | Routed correctly but a title or preserved sentence differs | Every title, sentence, cancellation rule and cleanup reproduced |
| Test coverage | ×2 | No injection tests | One flow covered | All four covered for failure, both pickers for cancellation, plus the tab-close case |
| Interface & readability | ×1 | Reporting logic re-added locally | Routed but contexts are generic | Flows only return; every boundary names itself |
| Assumptions & docs | ×1 | The stderr-to-notification move unexplained | Mentioned briefly | Explains why the picker's stderr was never visible |

## Out of scope

- The mechanism itself and the agent flows — already done.
- The terminal-facing subcommands — the next task.
