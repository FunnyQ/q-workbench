# Rust Rewrite of `q.workbench`

> **Status**: approved
> **Owner**: Q
> **Last updated**: 2026-07-31

## Overview

Replace the 14 zsh implementation scripts in the `q.workbench` Herdr plugin with a
single committed Rust binary that talks to Herdr's Unix-socket API directly, at
behavioural parity plus
six sanctioned deviations, cutting over in one revertible commit (with documentation
following immediately after).

## Goals

- One `bin/workbench` binary with `clap` subcommand dispatch replaces the 14
  implementation scripts under `scripts/`. One zsh script remains by design:
  `scripts/build.zsh`, which builds and installs the binary.
- Every Herdr call **made by this plugin's own code** goes over the socket. The
  `herdr` CLI is never spawned from `src/`. See the exemption below for the manifest.
- `serde` replaces every `jq` pipeline; TOML replaces `config.zsh`.
- The two duplicate agent-launch paths collapse into one shared module.
- Restart-in-place keeps working, on real `execvp` semantics.

## Non-goals

- **Replacing `gum` or `fzf`.** Both stay as external processes this round. A later
  leg may move the menu layer to `ratatui`.
- **Replacing `yazi`.** It is only ever started via a pane command.
- **Linux support.** The manifest declares `platforms = ["macos"]`; one arm64
  artifact suffices.
- **Any async runtime.** The socket serves exactly one request per connection, so a
  blocking `UnixStream` is enough. No `tokio`.
- **`events.subscribe`.** Verified to work and to hold a long-lived connection, but no
  current feature needs it.
- **CI-built binaries.** The release script is run by hand.
- **Behavioural redesign.** Menus, labels, glyphs, pane ratios and registry schemas
  stay as they are, except for the six sanctioned deviations below.
- **Replacing `herdr plugin pane open` in the manifest.** Five of the six actions open
  a popup pane by invoking that CLI command, and they keep doing so. This is an
  explicit, scoped exemption from the socket-only goal, justified below.

### The one CLI exemption

`herdr-plugin.toml` declares five actions whose `command` is
`["herdr", "plugin", "pane", "open", …]`. That line is a Herdr configuration value, not
code this plugin runs — Herdr spawns it, and it is Herdr's own mechanism for opening a
plugin popup. It stays.

Proxying it through the binary would be possible: `plugin.pane.open` exists as a socket
method and was verified working. It was rejected because the binary would first have to
learn its own plugin id — `q.workbench` when installed, `q.workbench-dev` when linked —
and no environment variable carrying that was verified. Adding an unverified lookup to
satisfy a purity claim is worse than scoping the claim honestly.

The claim this plan actually makes, and the one the final review checks, is: **no
process spawn of `herdr` or `jq` anywhere in `src/`.**

## Context

`q.workbench` is a Herdr plugin: pure zsh, no build step, ~1440 lines across 14
scripts plus 9 standalone zsh tests. It ships terminal-multiplexer actions — launching
AI agents into a 3-pane layout, fuzzy-picking projects and SSH targets, restarting a
stuck agent in place.

Three costs drove the rewrite:

1. **Every Herdr call shells out to the `herdr` CLI and is parsed with `jq`.** The CLI
   is a thin client over Herdr's socket API. Measured: the socket is ~4x faster
   (2.1–2.5 ms vs ~9.8 ms per call). That gap matters most on the fzf
   `--bind change:reload(...)` paths, which respawn a source script per keystroke.
2. **The two agent-launch paths are duplicate implementations of one flow.**
   `scripts/new-agent-popup.zsh:53-187` and `scripts/agent-launcher.zsh:75-205`
   differ only in whether the tab exists yet.
3. **The least maintainable code is jq, not zsh** — `scripts/project-registry.zsh:273-316`
   and `scripts/ssh-target-registry.zsh:66-78`.

Rust was chosen over TypeScript on Bun because
`std::os::unix::process::CommandExt::exec()` is a real `execvp`.
`scripts/agent-launcher.zsh:225` ends in `exec`, and restart-in-place depends on that
`exec` replacing the *launcher* subprocess rather than the pane's shell — killing the
agent's foreground process group then drops the pane back to its prompt instead of
destroying it. Bun has no process-replacement primitive at all (verified:
`process.execve` and `process.execv` are both `undefined`), which would have forced a
zsh trampoline.

Every measured figure from that research is reproduced in the table below and in
`tasks/_context/`; the research handoff it came from has been deleted, so this document
and the context files are now the only record. The 248 KB Herdr API schema is committed
at `tasks/_context/herdr-api-schema-protocol17.json`; regenerate it at any time with
`herdr api schema --json`.

## Requirements

### MVP

1. **Socket client** — a blocking `UnixStream` client covering the ~15 methods this
   plugin uses, behind a `HerdrClient` trait.
   - Acceptance: `workbench herdr ping` reports protocol 17; an integration test
     against a real `UnixListener` covers multi-chunk responses and connection close.
2. **TOML config** — every default from `scripts/config.zsh`, precedence user file →
   environment → built-in defaults.
   - Acceptance: unit tests mirror every still-applicable assertion in
     `tests/config.test.zsh`.
3. **Config migration** — `workbench config migrate` converts an existing
   `config.zsh` to TOML.
   - Acceptance: Q's real 4.1 KB config converts and the resulting TOML resolves to
     the same values.
4. **Project registry** — `scan|rescan|update|use|edit` with three-source discovery.
   - Acceptance: registry JSON byte-identical to the zsh version on the same inputs,
     **after normalising `generated_at`**. Both implementations stamp the current time,
     so the raw files can never match; the timestamp's format is covered by its own
     test.
5. **SSH registry** — `sync|list|get|use|remove` reconciled against the SSH config.
   - Acceptance: registry JSON and NUL-delimited `list` output byte-identical.
6. **Both pickers** — fzf-driven, with `reload`/`execute` bindings calling back into
   the binary.
   - Acceptance: each popup opens and is driven by hand through the dev plugin.
7. **Agent flows** — one shared menu module feeding both the popup and the in-pane
   launcher, plus restart and dashboard.
   - Acceptance: new-agent, new-worktree-agent and restart-in-place each run once
     through the dev plugin; restart is verified in a scratch tab.
8. **Cutover** — the manifest flip and the deletion of the zsh implementation land as
   **one** revertible commit. Documentation and release wiring follow in a second
   commit immediately after.
   - Acceptance: the installed plugin's six actions all work from the committed binary;
     `git revert` of the flip commit alone restores a working plugin (with temporarily
     stale docs, which is cosmetic).
   - Why two commits rather than one: what must be atomically revertible is the
     *behavioural* change. Documentation is a large, separate edit whose review is
     easier on its own, and stale docs after a rollback break nothing.

### Later

- **`ratatui` menu layer** — replaces `gum` and possibly `fzf`. Deferred: the picker's
  `--bind reload/execute` model would have to be rebuilt from scratch, and this round
  already changes the transport, the config format and the language at once.
- **`events.subscribe` driven features** — deferred: nothing needs them yet.
- **CI-built release artifacts** — deferred: one developer, one architecture.

## Tech decisions

- **Stack**: Rust (1.97.1 via mise), edition 2021. Crates: `clap`, `serde`,
  `serde_json`, `toml`, `anyhow`, `libc`, `time`. No async runtime.
- **Storage**: two JSON registries (unchanged paths and schemas), one TOML config, one
  small JSON state file for per-pane last harness/model.
- **Deployment**: the release binary is committed at `bin/workbench`.
  `herdr plugin install` is a real `git clone`, so the artifact lands with no install
  hook.
- **Conventions**: see `tasks/_context/shared.md`.

### Decision table

| # | Decision | Why |
|---|---|---|
| 1 | Behavioural parity is the acceptance bar | The zsh version is the reference implementation; any deviation must be a listed improvement |
| 2 | Internal consolidation is expected, not extra scope | Two agent paths become one shared module — writing them twice would take deliberate effort |
| 3 | `gum`, `fzf`, `yazi` stay | The picker's `--bind reload/execute` model would have to be rebuilt from scratch; not worth the risk this round |
| 4 | Option A — the plugin's own code talks only to the socket | CLI→socket mapping verified 1:1; the one gap (`plugin config-dir`) already has a literal fallback at `scripts/config.zsh:22`. The manifest's `herdr plugin pane open` lines are exempt, as scoped above |
| 5 | `HerdrClient` trait + `SocketClient` / `FakeClient` | Replaces the "fake `herdr` on `PATH`" test pattern, which has no equivalent once the CLI is gone |
| 6 | Config format is TOML | Matches `herdr-plugin.toml`; typed deserialisation into one struct |
| 7 | Binary is committed at `bin/workbench` | Install is a git clone, so a committed binary needs no install hook |
| 8 | Big-bang cutover: flip and deletion in one commit, docs in the next | No dual-config window; rollback is `git revert` of the flip commit |
| 9 | Development runs through a separate `q.workbench-dev` linked plugin | Lets every flow be driven for real without touching the installed plugin |
| 10 | Menu order unifies on harness → model → usage | The popup's order; it is the one Q drives daily |

### Sanctioned deviations from parity

1. **Menu order** in the in-pane launcher changes from harness → usage → model to
   harness → model → usage.
2. **`*_EXTRA_ARGS` become TOML string arrays.** Today `${=Q_CLAUDE_EXTRA_ARGS}`
   word-splits on spaces, so no single argument may contain one. A `Vec<String>`
   removes the limitation. Folded into the config task since it is the natural TOML
   shape, not extra work.
3. **Errors are reported consistently.** Today most failures are swallowed by
   `>/dev/null 2>&1`. Every fatal path reports a concrete cause on the channel that
   suits the subcommand: popup and in-pane flows notify, terminal-facing subcommands
   write to stderr. `project source` stays exempt — it runs once per keystroke.
4. **Protocol guard.** `ping` at startup; on a `protocol != 17` mismatch, notify that
   Herdr was upgraded and the binary needs rebuilding. Silent on success.
5. **Restart offers "use last combination".** A new first entry in the harness menu
   preselects what that pane last ran; picking anything else falls through to the
   normal menus.
6. **The worktree is created after every menu, not before.** Today `git worktree add`
   runs before the harness menu, so cancelling later leaves an orphaned worktree
   directory and branch. The Rust flow selects the branch first — the choice still
   drives every pane's `cwd` — but defers creation until all menus succeed. Found
   during plan review; it fixes an existing leak and is forced anyway by making the
   menu flow a decision module that creates nothing.

Deviations 3–5 live in the `polish` bucket and are individually cuttable. 1, 2 and 6
are forced by the consolidation, the config format and the decision-module boundary,
so they are not cuttable.

### How to cut one

Cutting a polish item is Q's call, and the dependency graph must not make it
impossible. `cutover/01` depends on the polish tasks, so to cut one:

1. Set that task's `Status` to `done` and add a line at the top of its Goal section
   saying it was cut and why.
2. Record the omission under **Known gaps** in `tasks/README.md`.
3. Note it in `cutover/03`'s findings.

Do not delete the task file — the record of what was skipped is the point. The
dependency then resolves and cutover proceeds.

## Verified facts this plan rests on

Measured on this machine, `herdr 0.7.5`, protocol 17.

| Fact | Evidence |
|---|---|
| Socket is ~4x faster than the CLI | 2.14 ms `ping` / 2.35 ms `pane.current` / 2.51 ms `pane.list` vs ~9.8 ms `herdr pane list` |
| Rust startup ≈ 3.6 ms, 431 KB hello-world | `cargo build --release`, Rust 1.97.1 via mise |
| Server closes the connection after one response | A second `ping` on the same connection gets no reply |
| Responses arrive in multiple chunks | `pane.list` returns ~12.9 KB split across chunks; clients must buffer to `\n` |
| `pane.send_input` `keys` accepts `enter` / `Enter` / `return`; rejects `cr` | Probed in a scratch tab; `cr` returns `{"code":"invalid_key"}` |
| `text` + `keys` in one `pane.send_input` call replaces `herdr pane run` | Same probe |
| `command[0]` accepts a relative executable | `./bin/probe` ran for both an `[[actions]]` and a `[[panes]]` entry; cwd is the plugin root |
| `events.subscribe` holds a long-lived connection | Returns `subscription_started`, then streams; `subscriptions` is an internally-tagged enum |
| `plugin install` is a git clone | `~/.config/herdr/plugins/github/q.workbench-41659eb013fa/.git` exists |

## Architecture

```
Cargo.toml
bin/workbench          # committed release artifact (arm64)
scripts/build.zsh      # cargo build --release && cp -> bin/workbench
dev/                   # q.workbench-dev manifest + run.zsh shim (dev only)
src/main.rs            # clap subcommand dispatch
src/config.rs          # replaces scripts/config.zsh; TOML
src/notify.rs          # notification.show helper
src/shell.rs           # shell_quote() for pane command strings
src/herdr/mod.rs       # HerdrClient trait, SocketClient, FakeClient
src/herdr/types.rs     # serde structs for the ~15 methods used
src/registry/project.rs
src/registry/ssh.rs
src/flows/agent.rs     # shared menu flow, both entry points
src/flows/picker.rs
src/flows/restart.rs
src/flows/dashboard.rs
src/state.rs           # per-pane last harness/model
```

Subcommands map onto today's scripts:

| Today | Rust |
|---|---|
| `new-agent-popup.zsh [worktree]` | `workbench agent popup [--worktree]` |
| `agent-launcher.zsh <pane> …` | `workbench agent launch <pane> …` |
| `build-agent-tab.zsh` | `workbench agent inject` |
| `restart-agent-popup.zsh` / `restart-agent.zsh` | `workbench agent restart` |
| `project-picker-popup.zsh` | `workbench project pick` |
| `project-picker-source.zsh` | `workbench project source [query]` |
| `project-registry.zsh {scan\|rescan\|update\|use\|edit}` | `workbench project {scan\|rescan\|update\|use\|edit}` |
| `ssh-picker-popup.zsh` | `workbench ssh pick` |
| `ssh-target-registry.zsh {sync\|list\|get\|use\|remove}` | `workbench ssh {sync\|list\|get\|use\|remove}` |
| `ssh-target-editor.zsh` | `workbench ssh edit` |
| `ssh-session.zsh` | `workbench ssh session` |
| `dashboard-launcher.zsh` | `workbench dashboard` |
| — | `workbench config migrate` |
| — | `workbench herdr ping` (diagnostic) |

## Bucketing

- **Strategy**: by layer, then by cutover phase.
- **Why**: the socket client and config are used by everything, so they must land
  first. Registries, pickers and agent flows are then largely independent of one
  another and can proceed in parallel. Cutover is deliberately isolated so it stays
  one revertible commit.

### Buckets

- **`foundation/`** — Cargo skeleton, socket client, config, migration, shell quoting,
  notifications, and the dev-plugin harness. Starts first; everything else depends on
  it.
- **`registry/`** — the two JSON registries. Depends only on the skeleton and config.
  Both are split at the discovery boundary, because the parsing rules are the part most
  likely to drift silently and are testable on their own. The project registry gets a
  third task for its interactive review; the SSH registry does not need one, being less
  than half the size.
- **`picker/`** — the project source, the project picker, and the three SSH pieces
  (session lifecycle, host editor, picker). The SSH work is split because the three
  have separate files, separate failure models and separate verification strategies.
- **`agent/`** — the shared menu flow, the popup, the in-pane launcher, restart, and
  the dashboard.
- **`polish/`** — the cuttable behavioural improvements. Runs after every entrypoint
  exists. The error-reporting sweep is split four ways — the mechanism plus the agent
  flows, the picker/SSH/dashboard flows, the stderr channel plus the project
  subcommands, then the SSH and config subcommands — because one task touching every
  module in the codebase is too large to execute or review in one pass. Four is where
  it stops: each piece is one module family plus its tests, and splitting further would
  produce tasks with no independently observable result.
- **`cutover/`** — manifest flip and zsh deletion, docs, and the final review.

## Task index

### `foundation/`

| Task | Depends on |
|---|---|
| `01-cargo-skeleton` | none |
| `02-herdr-socket-client` | `foundation/01` |
| `03-config-toml` | `foundation/01` |
| `04-config-migrate` | `foundation/03` |
| `05-shell-quote-and-notify` | `foundation/02` |
| `06-dev-plugin-harness` | `foundation/01` |

### `registry/`

| Task | Depends on |
|---|---|
| `01-project-discovery` | `foundation/03` |
| `02-project-store` | `registry/01` |
| `03-project-review` | `registry/02` |
| `04-ssh-discovery` | `foundation/03` |
| `05-ssh-store` | `registry/04` |

### `picker/`

| Task | Depends on |
|---|---|
| `01-project-picker-source` | `registry/02` |
| `02-project-picker` | `picker/01`, `agent/03` |
| `03-ssh-session` | `foundation/02`, `registry/05` |
| `04-ssh-edit` | `picker/03`, `registry/05` |
| `05-ssh-pick` | `picker/03`, `picker/04`, `foundation/05` |

### `agent/`

| Task | Depends on |
|---|---|
| `01-agent-flow-core` | `foundation/03` |
| `02-new-agent-popup` | `agent/01`, `foundation/05` |
| `03-agent-launcher` | `agent/01`, `foundation/05` |
| `04-restart-agent` | `agent/03` |
| `05-dashboard` | `foundation/03`, `foundation/05` |

### `polish/`

| Task | Depends on |
|---|---|
| `01-protocol-guard` | `foundation/05` |
| `02-error-reporting-core` | `agent/02`, `agent/03`, `agent/04` |
| `03-picker-error-reporting` | `polish/02`, `picker/02`, `picker/05`, `agent/05` |
| `04-terminal-error-reporting` | `polish/03`, `registry/02`, `registry/03`, `picker/01` |
| `05-ssh-config-error-reporting` | `polish/04`, `foundation/04`, `registry/05`, `picker/04` |
| `06-last-combination` | `agent/03`, `agent/04` |

### `cutover/`

| Task | Depends on |
|---|---|
| `01-manifest-flip` | `foundation/04`, `foundation/06`, `polish/01`, `polish/05`, `polish/06` |
| `02-docs` | `cutover/01` |
| `03-final-review` | `cutover/02` |

## Verification strategy

- **Unit and integration tests**: `cargo test`. `FakeClient` records calls, replacing
  the `herdr.log` assertions in `tests/new-agent-popup.test.zsh`. A real
  `UnixListener` covers the wire format.
- **Parity**: registries are checked byte-for-byte against the zsh output on the same
  inputs, before the zsh scripts are deleted.
- **Real use**: every flow is driven by hand through the linked `q.workbench-dev`
  plugin before cutover.
- **Restart-in-place** is exercised in a scratch tab, never a live one.

## Failure modes and rollback

| Risk | Mitigation |
|---|---|
| `exec` semantics break → restart destroys panes | The restart task verifies restart-in-place in a scratch tab before cutover |
| Shell-quoting bug mangles pane commands silently | Unit tests plus a live smoke test; this is a *new* failure surface the CLI never had |
| Herdr upgrade bumps the protocol | The protocol-guard task notifies instead of failing silently |
| Stale binary after an edit | `scripts/build.zsh` is the only sanctioned build path; `CLAUDE.md` documents that edits now need a rebuild |
| Cutover regresses something untested | `git revert` the flip commit; the docs commit is cosmetic and can follow or be reverted too |
| `gum` renders differently when spawned from Rust | Settled by the first menu task; falls back to passing an explicit `--width` |

## Open questions

- Real release binary size with `clap` + `serde` + `serde_json` + `toml` linked in.
  Estimated 2–3 MB, unmeasured. Settled by `foundation/01`.
- Whether `gum` renders identically when spawned from Rust rather than zsh. TTY
  inheritance is expected to be equivalent. Settled by `agent/01`.
