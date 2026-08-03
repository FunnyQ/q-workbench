# WORK-05: Route `tab new` and register the Herdr action

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/architecture.md`
> - `../_context/rubric.md`
>
> **Depends on**: work/04
> **Blocks**: work/06, work/07
> **Status**: done

## Goal

Make the layout-selector flow reachable: add the `tab new` subcommand to the CLI router and
register the Herdr action and popup pane that call it.

## Files to create / modify

- `src/main.rs` (modify) — the `Tab` command, its channel, its subcommand path, its router
  arm, and rows in the three enumeration tests
- `herdr-plugin.toml` (modify) — one `[[actions]]` entry and one `[[panes]]` entry

## Implementation notes

### What it calls

```rust
// src/flows/tab.rs
pub fn new(client: &dyn HerdrClient) -> FlowResult;
```

### `src/main.rs`

Four places must stay in sync for every leaf command, and each has a test that enumerates
them.

1. **The command tree.** Add a variant beside `Agent`, `Project`, `Ssh`, `Dashboard`,
   `Herdr`, and `Pane`:

   ```rust
   Tab {
       #[command(subcommand)]
       command: TabCommand,
   },

   #[derive(Debug, Subcommand)]
   enum TabCommand {
       /// Pick a tab layout, then open the agent popup for it.
       New,
   }
   ```

   `New` takes no arguments. There is deliberately no `--layout` and no `--worktree`.

2. **`channel()`** — the flow is popup-facing, so failures notify:

   ```rust
   Command::Tab { command: TabCommand::New } => Channel::Notification("New tab"),
   ```

   `Channel::Notification` implies `uses_herdr`, so `main` builds the socket client and runs
   the protocol guard before dispatching. That is what the flow needs.

3. **`subcommand_path()`** — `"tab new"`.

4. **The router arm** — mirror the shape the other notifying flows use:

   ```rust
   Command::Tab { command } => match command {
       TabCommand::New => {
           let client = client.context("Herdr client is required for a new tab")?;
           return flows::tab::new(client);
       }
   },
   ```

   The flow loads the config itself, so this arm must not load it.

Add rows to the three enumeration tests:

- `every_leaf_parses_with_all_supported_arguments` — `vec!["workbench", "tab", "new"]`.
- `non_agent_notifying_subcommands_carry_their_contract_titles` —
  `(vec!["workbench", "tab", "new"], Some("New tab"))`.
- `every_subcommand_selects_its_fixed_channel` —
  `(vec!["workbench", "tab", "new"], Channel::Notification("New tab"))`.

### `herdr-plugin.toml`

An action that opens a popup needs both halves. Add them next to the existing agent
entries, keeping the file's grouping: all `[[actions]]` first, then all `[[panes]]`.

```toml
[[actions]]
id = "new-tab"
title = "New tab"
contexts = ["workspace"]
command = ["herdr", "plugin", "pane", "open", "--plugin", "q.workbench", "--entrypoint", "new-tab", "--placement", "popup", "--width", "60%", "--height", "70%"]

[[panes]]
id = "new-tab"
title = "\u{f0a1e}  new tab"
placement = "popup"
command = ["./bin/workbench", "tab", "new"]
```

Two things to get right:

- `--entrypoint` must equal the `[[panes]]` `id`. A mismatch fails at runtime, not at load.
- TOML has no `\u{...}` escape. Write the pane title with the literal Nerd Font glyph
  followed by **two spaces**, exactly as the existing pane titles do — copy the two-space
  convention from `"󱚟  new agent"`. Any layout-ish glyph is fine; match the visual weight of
  the neighbouring entries.

Popup size `60% × 70%` matches the other agent popups. The plugin ships no keybinding.

## Acceptance criteria

- [x] `workbench tab new` parses and takes no flags.
- [x] `channel()` returns `Channel::Notification("New tab")` for it.
- [x] `subcommand_path()` returns `"tab new"` for it.
- [x] The router arm calls `flows::tab::new` with the client and loads no config of its own.
- [x] All three enumeration tests in `src/main.rs` carry a row for `tab new`.
- [x] `herdr-plugin.toml` has an action `new-tab` whose `--entrypoint` equals the id of a
      `[[panes]]` entry running `./bin/workbench tab new`.
- [x] The new pane title uses a glyph plus two spaces, like every other pane title.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo run -- tab --help` lists `new`; `cargo run -- tab new --layout x` is rejected
      by clap.
- [x] `git status --short -- src/main.rs herdr-plugin.toml` shows both paths dirty.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Entrypoint does not match the pane id, or the channel routes to stderr | Routes correctly but the arm loads the config or adds flags | Command, channel, path, and arm all correct; manifest halves agree |
| Test coverage | ×2 | No test rows added | One of the three enumeration tests updated | All three updated, and the parse test asserts the no-flag shape |
| Interface & readability | ×1 | Variant bolted onto an existing command | Separate but inconsistent with the neighbours | `Tab`/`TabCommand` mirror the existing command pairs exactly |
| Assumptions & docs | ×1 | Glyph or spacing convention broken silently | Title present but spacing off | Manifest entry matches the file's conventions, including two spaces after the glyph |

## Out of scope

- README and `config.example.toml`. A later documentation task owns both.
- Rebuilding `bin/workbench`. The closing task owns the rebuild, so the action will not
  work in a linked checkout until then — that is expected here.
- Any keybinding. The plugin ships none.
- A worktree variant of the action.
