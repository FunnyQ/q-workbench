# AGENT-04: `workbench agent restart`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: agent/03
> **Blocks**: polish/02, polish/06
> **Status**: done

## Goal

Relaunch the agent in its existing pane without tearing down the yazi and term side
panes, recovering the TTY that the killed harness may have left dirty.

## Files to create / modify

- `src/flows/restart.rs` (new) — the confirm popup and the restart sequence
- `src/main.rs` (modify) — wire `agent restart`

## Implementation notes

The full sequence — target resolution, focus walking, process-group kill with its
timings, the TTY reset string, and the re-injection parameters — is inlined in the
parity contract. Read it there. This section covers what is specific to the port.

### Two phases, two processes

The zsh version was two scripts: a popup that confirms, then a detached action that
does the work. Keep that shape, because the work phase kills a process group and has to
survive doing so — and the manifest routes this action through a popup pane, so the
confirming process is itself inside a pane that goes away.

Concretely:

- **`agent restart`** — runs in the popup. Shows `gum confirm` with the banner and
  flags from the parity contract. On rejection, exit 0 silently. On confirmation, spawn
  the worker and exit immediately, so the popup closes.
- **`agent restart-worker --pane <pane_id>`** — a hidden subcommand
  (`#[command(hide = true)]`) that does everything else. It resolves the target, kills
  the group, and re-injects. Taking the pane id as an argument means the worker does not
  re-read the plugin context from an environment it no longer shares.

Spawn it detached with a new session, so it belongs to no process group that the
restart can terminate and no controlling terminal that closing the popup can take away:

```rust
use std::os::unix::process::CommandExt;   // for pre_exec
use std::process::{Command, Stdio};

let mut cmd = Command::new(std::env::current_exe()?);
cmd.args(["agent", "restart-worker", "--pane", pane_id])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
// SAFETY: setsid is async-signal-safe and this closure runs between fork and exec.
unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }
cmd.spawn()?;
```

`setsid()` puts the child in a **new session and a new process group**. Without it the
worker inherits the popup's group, and either closing the popup or the worker's own
`kill -TERM -<pgid>` could take it out mid-flight — which would leave the agent killed
and never relaunched, the worst possible outcome for this feature.

Redirecting all three streams to `/dev/null` matters too: a worker still holding the
popup's TTY can block or corrupt the pane's display after the popup is gone.

`libc` is a dependency of this crate; it is also what the process-group kill below uses.

### The TTY reset

Codex can leave the pane in raw mode with the Kitty keyboard protocol enabled when its
process group is terminated. Two concrete symptoms: disabled ONLCR makes every newline
continue at the old column, and leftover Kitty CSI-u sequences make `gum` ignore arrow
keys.

The reset must run **inside the pane**, because this process does not own that TTY.
That is why it is prefixed onto the injected command rather than executed here.

That prefix is deliberately **not** shell-quoted — it is meant to be interpreted. Only
the launcher path and its arguments are quoted. Keep the boundary between the two
obvious in the code so nobody later "fixes" the unquoted part.

### Re-injection parameters

No tab rename, the current pane label as the fixed usage (so the usage menu is
skipped), no worktree step, and layout skipped.

Also pass the hidden `--restart` flag. It marks the launch as a restart, which a later
task uses to offer the previous harness and model. `--no-layout` must not be used as
that signal: it is a public option a manual launch can set too.

### Focus

A plugin action does not move keyboard focus. When invoked from a yazi or term pane,
focus must be moved to the agent pane before its menus open, by walking the four
directions with `pane.neighbor` until the neighbour matches the target. Both failure
messages in the parity contract must be preserved verbatim, along with their exit
codes: "no agent pane in this tab" exits 0, "could not focus" exits 1.

## Acceptance criteria

- [x] The confirm popup uses the parity contract's exact banner, `--affirmative
      "Restart"`, `--negative "Cancel"`, and the four colour flags with the same values;
      rejecting exits 0 with no side effect.
- [x] `agent restart-worker --pane <id>` exists as a hidden subcommand and receives the
      pane id as an argument rather than re-reading the plugin context.
- [x] The worker is spawned with `setsid()` in `pre_exec` and all three streams
      redirected to `/dev/null`; `agent restart` exits immediately after spawning it.
- [x] The target pane is the focused pane when it has an agent, otherwise the first
      agent pane in the same tab.
- [x] No agent pane in the tab → notify and exit 0. No matching neighbour direction →
      notify and exit 1. Both message strings preserved.
- [x] The kill guard holds: only when the foreground process group is present,
      non-zero, and different from the shell pid.
- [x] TERM, then up to 50 polls at 100 ms, then KILL, then a 300 ms settle.
- [x] The injected command carries the unquoted `stty sane; printf …;` prefix followed
      by the quoted launcher invocation.
- [x] Re-injection passes no tab id, the current label as fixed usage, no worktree,
      layout skipped, and the hidden `--restart` flag.
- [x] The pane survives: after restart, the yazi and term panes are still present.

## Verification

- [x] `cargo test` — with `FakeClient`, assert target resolution for both cases and
      both failure paths, including exit codes and message strings
- [x] `cargo test` — assert the injected command string: prefix literal, launcher path
      and arguments quoted, `keys: ["enter"]`
- [x] **Live test in a scratch tab** — this is the critical one, and what the `exec`
      semantics exist for:
      1. build an agent tab in a scratch tab, let the harness start
      2. run `agent restart` against it
      3. confirm the agent pane returns to a prompt and then relaunches, and that the
         yazi and term panes are still there
      4. repeat with codex specifically, and confirm arrow keys work in the new menus
         (this is what the Kitty reset is for)
      5. close the scratch tab
- [x] `cargo test` — an automated survival test: spawn the worker entry point with a
      long-running stand-in for its body, send `SIGTERM` to the **parent's** process
      group, then assert the worker is still alive and its session id differs from the
      parent's (`getsid`)
- [x] Manual: confirm the worker survives the real restart — it must not die with the
      pane's foreground group or when the popup closes
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | The pane is destroyed, or the worker dies with the group it kills or with the popup | Restart works but the TTY reset or a failure message differs | Pane and side panes survive, TTY clean afterwards, every message and exit code matches |
| Test coverage | ×2 | No live test | Unit tests only | Unit tests plus the full live scratch-tab sequence, including the codex arrow-key check |
| Interface & readability | ×1 | Quoted and unquoted parts of the injected command indistinguishable | Separated but undocumented | Clear boundary, with a comment saying why the prefix is unquoted |
| Assumptions & docs | ×1 | No note on why the worker must be detached | Mentioned in passing | `setsid`, the null stdio, TTY quirks and the kill guard all explained |

## Out of scope

- Preselecting the previous harness and model — a later polish task adds that entry.
- Any change to how the launcher itself builds the layout.
