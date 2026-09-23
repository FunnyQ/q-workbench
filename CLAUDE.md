# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A **Herdr plugin** (`q.workbench`) implemented as a Rust binary. It launches AI agents in structured tab layouts, picks projects and SSH targets, and restarts agents in place.

`herdr-plugin.toml` registers the actions and popup panes. Most entries call the committed `bin/workbench` artifact. Run `zsh scripts/build.zsh` after changing Rust or embedded shell scripts, and commit the rebuilt binary with a release. A linked checkout still runs that artifact, so forgetting the rebuild is the usual cause of code and behavior disagreeing.

## Commands

```zsh
cargo test
cargo clippy -- -D warnings
zsh scripts/build.zsh
```

`cargo test` covers the CLI, socket protocol, registries, pickers, and launch flows. Tests use `FakeClient` for ordered Herdr responses and temporary directories for filesystem state. Keep subprocess boundaries injectable when a flow needs `gum`, `fzf`, `git`, `ssh`, or `zoxide`.

## Architecture

### CLI and configuration

`src/main.rs` is the single command router. Herdr actions and internal reinjections use the same `agent`, `project`, `ssh`, `dashboard`, `herdr`, and `pane` command tree, so add behavior there before wiring a manifest entry.

`src/config.rs` loads TOML from the per-plugin config path. The config has two array-of-tables sections: `[[tab_layouts]]` describes tab layouts and their panes, while `[[agents]]` describes agents and their options. TOML arrays preserve argument boundaries, including values that contain spaces. Keep bypass flags opt-in through an agent's `extra_args` array; nothing adds them unconditionally.

Omitting a pane's agent, a pane's option, or a layout's tab label makes the launcher ask for that choice at launch time. Select a layout with `--layout <name>`; launches without the flag use `default_tab_layout`.

The new-tab menu lists layouts in config order — `default_tab_layout` selects what a flagless launch opens, it does not move the row — and always offers a blank tab, as its last row, whether or not the config declares one. `config::BLANK_LAYOUT_NAME` reserves the name: a config layout under it replaces the built-in body and keeps the last slot. Because that row is always added, the menu draws even when the config declares a single layout.

A layout may declare no agent pane, one, or several, at any position. Each unpinned agent pane runs its own harness and model menu, in the order the panes are written, and the usage menu runs once afterwards for the tab. A layout with no agent pane runs none of them; it asks for a plain tab name instead, where submitting nothing keeps the layout's own label and escaping cancels. Reach the agent panes through `TabLayout::agent_panes`; nothing may assume `panes[0]` runs a harness.

All validation runs at config load, before the first socket call. An agent's `kind` is the exception the plugin cannot check: the list of kinds is Herdr's, so a mistyped one is only refused by `agent.start`, which closes the tab it was building and reports. Layout construction is not atomic either — the popup path closes its tab on failure and the in-pane path splits pane by pane — so deferred validation could leave a half-built layout on screen.

### Herdr socket contract

`src/herdr/mod.rs` talks directly to `HERDR_SOCKET_PATH`. Herdr accepts exactly one request per connection and closes the connection after its response. Open a fresh `UnixStream` for every call; connection reuse hangs rather than saving work.

A refused call carries `ErrorResponse` as the error itself rather than a formatted string, so a caller can tell one refusal from another by `code` without matching on prose Herdr is free to reword. Its `Display` is the message every existing context chain already prints.

Requests and responses are newline-delimited JSON. A response can arrive in multiple chunks, so buffer reads until the first newline before parsing. Never assume one `read` contains the full response.

`pane.send_input` replaces the old run-command path. Its `keys` field uses Herdr's key vocabulary: use `"enter"` to submit text. `"Enter"` and `"return"` are accepted, but `"cr"` is rejected. Sending `text` with `keys: ["enter"]` types into the pane's interactive shell and executes it.

`layout.apply` builds a whole tab — structure, labels, cwd, env — from one tree in one call, and it is atomic, so a failure leaves no half-built tab behind. It does not adopt existing panes: handing it a tab id rebuilds that tab, losing scrollback and issuing fresh pane and tab ids, so it can create a tab but never edit the one the caller lives in. Its reply carries Herdr's pane ids and none of our pane names, so in-order traversal position is the only link back.

`agent.start` starts a harness Herdr recognises, which is what makes the pane an agent Herdr can see and report sessions for. It requires a pane already sitting at its interactive shell prompt — a pane `layout.apply` created moments ago is refused with `agent_pane_busy` until its shell profile finishes loading, which on a real machine takes seconds, so that one code is retried and every other refusal reports at once. Its `name` is `[a-z][a-z0-9_-]{0,31}`, which no pane label satisfies once it carries a glyph, a capital or a space; `agent_name` reduces the label rather than handing it over, because Herdr rejects the whole call with `invalid_agent_name`. It derives the executable from `kind` and appends only the args — so it can stand in only for an agent whose `command` is that bare executable. An option that overrides `command`, an agent command carrying fixed arguments, and an agent command naming a different binary all keep the typed-argv path; otherwise the same config would launch two different argvs depending on which path built the pane.

`pane.report_agent_session` is how a session id reaches Herdr's snapshot. `MINIMUM_PROTOCOL` is 22 for these three methods: the constant now also rises when the plugin starts needing a method older servers never had, not only when a protocol drops something the plugin reads.

### Agent launch and restart

The popup gathers any choices that the selected layout leaves open before it creates the final layout. The launcher deliberately defers non-agent panes until every menu is complete. Splitting sooner resizes the agent pane while menus are drawing, and a chosen worktree must determine the cwd of every pane.

The popup path builds its tab with a single `layout.apply`: `build_layout_tree` folds the layout's flat pane list into a tree, and the reply names every pane at once. Only that call is atomic. Focusing the tab and starting what each pane runs happen afterwards, one call at a time, so a failure there closes the tab it built and says so — an `agent.start` that never reaches ready would otherwise strand a tab with no agent in it. The focus comes before the starts, because `agent.start` blocks until its harness is ready and the caller would sit on the old tab until then.

The in-pane path keeps its `pane.split` loop, and the two construction paths are a deliberate cost of the atomic popup build. `layout.apply` destroys the panes it is given, so it would kill the launcher before its `exec` ever ran, and `agent.start` needs a pane already at its shell prompt — the launcher itself is what occupies that prompt. Neither can serve a pane the plugin is running inside.

The injected launcher ends with `exec` so the harness replaces the launcher process. Restart depends on this: terminating the foreground harness returns the pane to its interactive shell, then the restart worker can inject a new launcher without destroying the side panes.

Only the root agent pane is reachable that way. `agent launch` replaces the process of the pane it runs in, so it treats that pane as the tab root and rejects a layout whose root is not an agent pane; such layouts open from the popup instead. Every pane the plugin does not `exec` into is started after it exists: through `agent.start` when the agent declares a `kind` whose bare executable is its whole command, and otherwise by typing its quoted argv into the pane, the way a command pane is started.

Restart state is per pane id, and each record carries the layout pane it was launched as. Without that name a side agent pane would relaunch under the first agent pane's pin. Bump `STATE_VERSION` when the record's shape changes; a stale file is discarded whole rather than read into the new shape. The one exception is a field whose absence already means the right thing: `effort` arrived without a bump, because a record missing it replays the option's default effort, and a bump would have dropped every open pane's session fallback.

The model menu is the one menu not drawn by `gum`. `gum choose` takes no key bindings and spends left and right on paging, so `GumMenu::choose_model` puts the terminal in raw mode itself and steps each row's effort with left and right. An option opts in by naming a default `effort`. The agent's `efforts` lists the levels, and an option's own `efforts` replaces that list for a model whose levels differ. `effort_args` carries the level to the harness, after the option's args and before `extra_args`.

The restart menu always draws all three rows — resume, fresh, cancel — because deciding whether a session id exists would mean a socket call, and the popup pane dies the moment the menu returns. Fresh leads the rows because it is the only one that always does what it says. Resolution is the detached worker's: it reads Herdr's `agent_session` first, the state record's own `session` second, and sweeps the harness's session files third, and a Resume that ends up with nothing degrades to a fresh start reported through `Outcome::Notice` rather than starting over in silence. Two things end up with nothing besides an absent id: an `agent_session` whose `kind` is `path` rather than `id`, which is a transcript and not something a harness resumes, and an agent whose kind takes no resume argument at all. The worker reads the session **before** it kills the harness, because Herdr binds a reported session to the agent it detected and clears it with that agent.

That stored session id comes from a reporter spawned detached for every agent pane: `launch` spawns its own just before `exec`, since after `exec` the launcher no longer exists, and `start_pane` spawns one for every pane the plugin does not `exec` into. Without the second the popup — the main entry point — would record nothing, and a `pair` layout's side agent could never resume. The reporter polls the kind's session files for one written under this pane's cwd after launch, which is a heuristic: two agents sharing a cwd — what the `pair` layout does — can be credited each other's session, and the launch timestamp only narrows that window. It also gives up after a minute, which on its own would have made resume useless for the ordinary case: claude writes no transcript until its first turn, so a pane opened and prompted an hour later recorded nothing. That is why the worker's third source sweeps the same session files at restart time, bounded by the record's launch stamp instead of a fixed window. The reporter survives because Herdr's snapshot is worth populating early even when nothing reads it back. Herdr's own `agent_session` wins on read, so installing Herdr's integration later improves accuracy without touching either fallback.

Restart injection crosses a shell boundary. `pane.send_input` sends command text, then the pane's shell parses it. Build reinjection commands with `src/shell.rs`; quote every executable path and argument separately so spaces, quotes, and shell metacharacters cannot change argv.

Codex can leave the TTY and Kitty keyboard protocol dirty after termination. Keep the restart reset sequence before reinjection, or gum can render in the wrong column and ignore arrow keys.

### Popup cwd

A popup's current directory is not a reliable project directory. Resolve project context from Herdr's session and pane data instead of trusting process cwd. This matters most for worktree creation and project actions launched from popup panes.

### Registries

`src/registry/project.rs` and `src/registry/ssh.rs` own the two version-1 JSON stores. Writes replace the complete registry atomically. Preserve stable ordering and existing source metadata when changing either schema.

`project review` is the popup half of `project scan` and `project rescan`: their guards are complementary, so it picks by whether the registry file exists and reports through a notification, while the two terminal commands keep reporting on stderr. A dismissed review menu carries `ReviewCancelled` so the popup can end clean without matching on error text.

Project discovery merges Claude sessions, Codex rollouts, and a `.git` sweep of `projects_root`. Canonicalization resolves each candidate to its Git root and rejects temp-directory paths. Keep the temp-dir filter: test sandboxes and transient worktrees must not leak into the real registry.

SSH sync reconciles configured hosts through `ssh -G`. Removing a config-sourced entry hides it; removing a manual entry deletes it. A successful session stamps usage, and its dedicated tab closes on every connection exit path.

### fzf pickers

Both pickers feed fzf NUL-delimited, multi-line records with a tab-separated payload. Newlines belong to the visible row, so line-delimited records corrupt selection boundaries. Keep `--read0`, the payload delimiter, and positional parsing together.

The project picker emits three sources in a fixed order: registry entries ranked by last use, then one `zoxide` fallback, then a live sweep of `projects_root` for projects the registry does not hold. The zoxide lookup and the sweep both wait for `DISCOVERY_MINIMUM_QUERY` characters, so the opening draw costs one registry read. The sweep keeps the registry lean — a directory earns an entry when it is picked, not when it is found.

The sweep counts a directory as a project when it holds `.git` or one of `project_markers`, and stops there rather than walking its contents. Both halves matter. A marker is what admits a project that has no checkout, because depth cannot separate a project from a directory that holds projects — a real projects root nests them unevenly. Stopping is what keeps the sweep affordable: `discover_filesystem_projects`, which walks through a project to find one nested inside it, costs ~1.3 s against a real projects root versus ~11 ms, and the picker reloads on every keystroke. Markers belong to `discover_project_checkouts` alone; `project update` must keep writing only what `.git` finds.

Plain enter opens the agent layout in a new workspace; alt-enter leaves it plain. Existing workspaces are focused without rebuilding their tabs.

fzf owns the alternate screen while edit bindings run. Clear it before drawing a `gum` editor, then let fzf reload its source after the editor exits.

## Conventions

- Keep Herdr protocol types in `src/herdr/types.rs` and expose typed client helpers from `src/herdr/mod.rs`.
- Route popup failures through notifications and durable CLI failures through stderr. Treat cancellation as a clean outcome.
- Keep comments focused on ordering, protocol, terminal, and quoting reasons that are not obvious from the code.
- Use Rust filesystem operations that match the registry contract. Remove only paths the operation is meant to delete.
- Rebuild `bin/workbench` before testing the linked plugin or preparing a release.
