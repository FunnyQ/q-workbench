# Shared context

> All tasks reference this. Decisions here override anything inferred from the codebase.

## Project at a glance

`q.workbench` is a Herdr plugin implemented as a single Rust binary. It launches AI agents
into structured tab layouts, picks projects and SSH targets, and restarts agents in place.
`herdr-plugin.toml` registers the actions and popup panes; most entries call the committed
`bin/workbench` artifact. The repository root is the crate root.

This plan adds one action: a popup that lists the configured tab layouts, then runs the
existing agent popup flow with the chosen layout.

## Tech stack

- **Language**: Rust 2021, one binary (`src/main.rs`) over one library (`src/lib.rs`).
- **CLI**: `clap` derive. `src/main.rs` is the single command router.
- **Config**: TOML via `serde` + `toml`, loaded by `src/config.rs`.
- **IPC**: newline-delimited JSON over the Unix socket at `HERDR_SOCKET_PATH`.
- **Menus**: the `gum` binary, driven as a subprocess.
- **Pickers**: the `fzf` binary. Not touched by this plan.
- **Platform**: macOS arm64 only. The committed binary is a Mach-O arm64 artifact.

## Code style

- Match the style of the surrounding file. Do not reformat neighbouring code.
- Comment *why*, never *what*. The comments worth writing explain ordering, protocol,
  terminal, and quoting reasons that are not obvious from the code.
- Errors carry context with `anyhow::Context`. Popup-facing failures route through
  `FlowError` and a notification; terminal-facing failures go to stderr.
- Cancellation is a clean outcome (`Outcome::Cancelled`), never an error, and never
  notifies.
- Prefer a longer function over an indirection used once.
- Keep visibility as narrow as the callers require: `pub(crate)` before `pub`.
- Authoritative check (for verification only): `cargo clippy -- -D warnings` must be clean.

## File / directory layout

```
src/
├── main.rs          CLI router: Command → flows::*
├── config.rs        TOML load + all validation
├── shell.rs         quoting for commands sent through a pane's shell
├── state.rs         per-pane last-agent record
├── notify.rs        notification.show helper
├── flows/
│   ├── mod.rs       module list + shared helpers (nonempty_env, terminal_size, FlowError)
│   ├── agent.rs     agent choice menus, popup, launch, inject
│   ├── restart.rs   restart-in-place worker
│   ├── picker.rs    fzf project / ssh pickers
│   ├── ssh.rs       ssh session flow
│   ├── layout.rs    even-out-panes (split-ratio maths; unrelated to tab layouts)
│   └── dashboard.rs dashboard launcher
├── herdr/
│   ├── mod.rs       typed socket client + FakeClient
│   └── types.rs     protocol request/response types
└── registry/        project + ssh JSON registries
```

New flow modules go under `src/flows/` and are declared in `src/flows/mod.rs`. Unit tests
live in a `#[cfg(test)] mod tests` (or a named module such as `mod popup`) at the bottom of
the file they test. There is no `tests/` directory.

Note the name collision: `src/flows/layout.rs` is the pane split-ratio flow behind
`pane even`. Tab layouts are a config concept in `src/config.rs`. This plan touches the
latter and must not touch the former.

## Herdr socket contract

Repeated here because a wrong assumption here costs a hang, not a compile error:

- Herdr accepts exactly one request per connection and closes it after the response. Open
  a fresh `UnixStream` for every call.
- Responses can arrive in multiple chunks. Buffer until the first newline before parsing.
- `pane.send_input` uses Herdr's key vocabulary: `"enter"` submits. `"cr"` is rejected.
- Sending `text` with `keys: ["enter"]` types into the pane's interactive shell, so every
  executable path and argument must be quoted separately via `src/shell.rs`.

## Popup constraints

- A popup's process cwd is the plugin checkout, not the project. Resolve project context
  from Herdr's session and pane data (`adopt_invoking_pane_cwd`), never from process cwd.
- Load and validate the config **before the flow issues its first request**, so a broken
  config is reported without leaving anything half-built on screen. This invariant is
  about the flow's own calls. `main` already builds the socket client and runs the
  protocol-guard `ping` before dispatching any notifying command; that is the existing
  contract for every action and nothing in this plan changes it.
- `gum` draws its UI on stderr whenever stdout is not a terminal. Inherit stderr or the
  menu renders nowhere.
- A non-zero `gum` exit means the user cancelled: `Ok(None)`, never an error.

## Commit & branching style

- Base branch: `main`. Branch before committing; do not commit straight to `main`.
- Commit with `chronicle:commit`. Do not hand-roll `git commit`.
- Commit subjects follow the repo's existing emoji + conventional style, for example
  `✨ feat: config-driven tab layouts and agents` or `📖 docs: ...`.
- No CHANGELOG entry and no version bump in this plan.

## Verification baseline

```zsh
cargo test                      # unit tests, all in-crate
cargo clippy -- -D warnings     # must be clean
zsh scripts/build.zsh           # rebuilds the committed bin/workbench
```

`zsh scripts/build.zsh` is only needed by the closing task and by anyone testing the linked
plugin. A linked checkout runs the committed `bin/workbench`, so skipping the rebuild is
the usual cause of code and behaviour disagreeing.

## Decisions frozen during interview

- **New `tab new` subcommand** — the layout selector is its own command, not a flag on
  `agent popup`.
- **`label` + `icon` on `[[tab_layouts]]`** — optional, rendered by the existing
  `render_label`, falling back to `name`.
- **No worktree support in `tab new`** — worktree launches keep using the existing
  `new-worktree-agent` action.
- **Menu order: default layout first, then config order** — `default_tab_layout` is hoisted
  to the top; the rest keep the order they appear in `config.toml`.
- **A single configured layout skips the menu.**
- **Cancelling the layout menu is silent** — `Outcome::Cancelled`, no notification.
- **Menu primitives move to `src/flows/menu.rs`** — shared by `agent.rs` and the new
  `tab.rs`.
- **Empty `label`/`icon` and duplicate rendered menu labels are load-time errors** —
  mirroring the existing agent-label validation.
- **Delivery includes README, `config.example.toml`, and a rebuilt `bin/workbench`.**
