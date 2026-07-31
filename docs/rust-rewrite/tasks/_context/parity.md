# Parity contract

> Behavioural parity with the zsh version is the acceptance bar. This file inlines the
> literals, sequences and schemas that must survive the port, plus the six sanctioned
> deviations. If something is not listed here and not listed as a deviation, keep it
> exactly as the zsh source has it.

## The six sanctioned deviations

Everything else must match. These six are approved changes:

1. **Menu order in the in-pane launcher** changes from harness → usage → model to
   harness → model → usage. The popup already used harness → model → usage; both
   entry points now share one flow.
2. **Extra-args settings become string arrays.** Today `${=Q_CLAUDE_EXTRA_ARGS}`
   word-splits on spaces, so no single argument may contain one. In TOML they are
   `Vec<String>` and the limitation disappears.
3. **Every fatal path reports a concrete cause, on the channel that suits the
   subcommand.** Today most failures are swallowed by `>/dev/null 2>&1`. Popup and
   in-pane flows notify; terminal-facing subcommands write to stderr — see the message
   tables below for which is which. `project source` stays exempt: it runs once per
   keystroke and must not gain output of any kind.
4. **Protocol guard**: `ping` at startup; on `protocol != 17`, notify that Herdr was
   upgraded and the binary needs rebuilding. Silent on success.
5. **Restart offers "use last combination"** as an extra first entry in the harness
   menu, preselecting what that pane last ran. Picking anything else falls through to
   the normal menus.
6. **The worktree is created after every menu, not before.** Today the worktree step
   runs first and calls `git worktree add` immediately, so cancelling at the harness,
   model or usage menu leaves an orphaned worktree directory and branch behind. The
   Rust flow **selects** the branch first — the step still comes first, because the
   choice drives every pane's `cwd` — but creation is deferred until all menus have
   succeeded. This is a fix for an existing leak, and it is forced anyway by making the
   menu flow a pure decision module that creates nothing.

Items 3–5 are individually cuttable. Items 1, 2 and 6 are forced by the consolidation,
the config format and the decision-module boundary.

## Nerd Font glyphs — exact codepoints

Menu labels carry Nerd Font glyphs and double as pane and tab labels. Getting these
wrong is invisible in a diff and obvious on screen. Use the codepoints, not a copied
glyph.

| Where | Literal (codepoint + spacing) |
|---|---|
| Harness menu title | `U+F169F` + two spaces + `Launch Agent` |
| Harness option 1 | `U+F15CE` + two spaces + `claude code` |
| Harness option 2 | `U+0EE0D` + two spaces + `codex` |
| Harness option 3 | `U+F169F` + two spaces + `opencode` |
| Model menu title | `U+F09D1` + two spaces + `claude code` |
| Usage menu title | `U+0F27B` + two spaces + `Usage` |
| Usage option 1 | `U+0F442` + two spaces + `discuss` |
| Usage option 2 | `U+0F4AF` + two spaces + `review` |
| Usage option 3 | `U+0EAD8` + two spaces + `debug` |
| Usage option 4 | `U+F19B9` + two spaces + `let me write…` (U+2026, not three dots) |
| yazi pane label | `U+F0968` + two spaces + `Files` |
| term pane label | `U+0F489` + two spaces + `term` |
| Injected agent pane label | `U+F169F` + two spaces + `agent` |
| Project picker row prefix | `U+F024B` + two spaces + project name |
| SSH tab label | `U+F08A9` + two spaces + target |
| Project picker pinned tab | `U+F09D1` + two spaces + `main` |
| Dashboard tab label | `U+0EACD` + two spaces + `Dashboard Launcher` |
| Restart confirm title | `U+F002A` + two spaces + `Current session will end` |
| "use last" harness entry | `U+F0709` + two spaces + `use last: <harness>`, and for claude ` · <model label>` |

Known inconsistency in the source: `scripts/agent-launcher.zsh:183` uses **one** space
after `U+F09D1` in the model banner, while `scripts/new-agent-popup.zsh:106` uses two.
The unified flow uses **two**.

Menu options are rendered with a leading pad of spaces for centering. The pad is
stripped from the selection before the value is used
(`${x#"${x%%[![:space:]]*}"}`) — **the glyph is kept**, because the stripped label
becomes the pane and tab label.

## Message strings

Two reporting channels, and which one a subcommand uses depends on whether it has a
durable place to print:

- **Notification** — flows that run in a popup or a pane that disappears. The user
  would never see stderr.
- **stderr** — subcommands run from a terminal, where the text stays on screen.

### Notifications

`position` is `bottom-right` on all of them. The popup cleanup also passes
`sound: "none"`.

| Where | Title | Body |
|---|---|---|
| Popup, failure after the tab exists | `Agent tab failed` | `The incomplete tab was closed.` |
| Project picker, registry missing | `Project picker` | `project picker: registry not found: <path>` |
| Project picker, query resolved to nothing | `Project picker` | `project picker: project not found: <query or path>` |
| Restart, no agent pane in the tab | `Restart agent` | `No agent pane in this tab to restart.` (exit 0) |
| Restart, no neighbour direction matched | `Restart agent` | `Could not focus the agent pane.` (exit 1) |
| Dashboard, workspace label not found | `Dashboard Launcher` | `Workspace '<label>' was not found.` — the configured label interpolated, in single quotes |

### stderr

Each line is written to stderr followed by the stated exit code.

| Subcommand | Text | Exit |
|---|---|---|
| project registry ops | `project-registry: registry does not exist: <path>` | 1 |
| project registry ops | `project-registry: invalid registry: <path>` | 1 |
| project scan | `project-registry: registry already exists: <path>` | 1 |
| project scan/rescan | `project-registry: no projects found` | 1 |
| project scan/rescan | `project-registry: cancelled; registry not written` | 1 |
| project scan/rescan | `project-registry: nothing selected; registry not written` | 1 |
| project use | `project-registry: project path is required` | 1 |
| project use | `project-registry: invalid project path: <path>` | 1 |
| project edit | `project-registry: absolute project path is required` | 1 |
| project edit | `project-registry: project is not registered: <path>` | 1 |
| project edit | `project-registry: edit cancelled; registry not written` | 1 |
| project edit | `project-registry: invalid visibility; registry not written` | 1 |
| project edit, on success | `project-registry: edited <path>` — **stdout** | 0 |
| project update, on success | `project-registry: updated <path>` — **stdout** | 0 |
| project scan/rescan, on success | `project-registry: wrote <path>` — **stdout** | 0 |
| ssh edit | `No SSH target selected.` | 1 |
| ssh edit | `Invalid SSH alias: <alias>` | 1 |
| ssh edit | `Invalid HostName: <hostname>` | 1 |
| ssh edit | `Invalid SSH user: <user>` | 1 |
| ssh edit | `Invalid SSH port: <port>` | 1 |
| ssh edit | `SSH alias already exists: <alias>` | 1 |
| ssh edit, on success | `Added SSH config: <alias>` — **stdout** | 0 |

The project picker's two messages move from stderr to a notification, keeping their
exact text as the body. That is a fix, not a drift: the picker runs inside a popup
pane, so its stderr was never visible to the user.

### Everything the zsh version never had a message for

Several failure paths simply exited. Where the table above specifies no text, the
format is:

```
<subcommand path>: <chained cause>
```

on stderr, exit 1 — for example `ssh sync: cannot read /Users/q/.config/ssh/config:
permission denied`. This applies to `ssh sync|list|get|use|remove`, `config migrate`,
`herdr ping`, and any fatal path in a listed subcommand that the table does not name.

The `gum is required` and `jq is required` guards disappear: `jq` is gone, and a
missing `gum` surfaces as a spawn failure with the same effect.

The old `Usage: …` lines for the two registries disappear too — `clap` produces its own
usage output for an unknown or malformed subcommand.

## Restart confirmation prompt

`gum confirm` with the banner as its prompt and these exact flags:

```
--affirmative "Restart"
--negative "Cancel"
--selected.background 214
--selected.foreground 235
--unselected.background 237
--unselected.foreground 223
--padding "1 <content margin>"
```

The banner is `gum style --border rounded --padding '1 3' --width <content width>
--bold '<U+F002A>  Current session will end' '' '<dim>The agent will relaunch in
place.</dim>'`, where the dim line is itself produced by `gum style --foreground 240`.

`content width` is 44, clamped to `cols - 4`; `content margin` is
`(cols - content width - 2) / 2`, floored at 0.

## Config defaults

Every one of these must resolve identically after the TOML port. The names on the left
are the current environment/zsh variable names; keep them as the environment-override
names.

| Setting | Default |
|---|---|
| `Q_DASHBOARD_WORKSPACE` | `personal-assistant` |
| `Q_CLAUDE_EXTRA_ARGS` | empty |
| `Q_CODEX_EXTRA_ARGS` | empty |
| `Q_PROJECT_REGISTRY_FILE` | `$HOME/.local/state/herdr-projects/registry.json` |
| `Q_PROJECTS_ROOT` | `$HOME/Projects` |
| `Q_SSH_REGISTRY_FILE` | `$HOME/.local/state/ssh-targets/registry.json` |
| `Q_SSH_CONFIG_FILE` | `$HOME/.config/ssh/config` |
| `Q_SSH_HISTORY_FILE` | `$HOME/.zsh_history` |

Model menu defaults — order drives the menu, the map resolves a label to a `--model`
value, and the args map adds per-label flags:

```
order      = ["Opus", "OpusPlan (Sonnet)", "CCR", "Fable 5"]
models     = { "Opus" = "claude-opus-4-8",
               "OpusPlan (Sonnet)" = "opusplan",
               "CCR" = "CCR",              # not a model — dispatches to `ccr code`
               "Fable 5" = "claude-fable-5" }
model_args = { "OpusPlan (Sonnet)" = ["--effort", "medium"] }
```

**The bypass flags stay opt-in.** `--dangerously-bypass-approvals-and-sandbox` and
`--dangerously-skip-permissions` belong only in the extra-args settings, never as a
default and never behind a dedicated boolean. `tests/new-agent-popup.test.zsh:78-99`
pins both the off and on states; the Rust tests must pin the same.

## Launch command assembly

| Harness | Command |
|---|---|
| codex | `codex` + codex extra args |
| opencode | `opencode` |
| claude, model `CCR` | `ccr code` (no model flag, no extra args) |
| claude, any other model | `claude --model <value>` + per-label model args + claude extra args |

## The agent tab layout — exact call sequence

`tests/new-agent-popup.test.zsh:61-70` asserts this sequence for the popup path.
Reproduce it method-for-method over the socket. `<cwd>` is the resolved project
directory, `<label>` the chosen usage label.

1. `tab.create` — `label = <label>`, `cwd = <cwd>`, `env = {Q_NO_BANNER: "1"}`,
   `focus = false`, `workspace_id` only when `HERDR_WORKSPACE_ID` is set
2. `pane.rename` — agent pane → `<label>`
3. `tab.rename` — tab → `<label>`
4. `pane.split` — from the agent pane, `direction = right`, `ratio = 0.38`,
   `cwd = <cwd>`, `env = {Q_NO_BANNER: "1"}`, `focus = false`
5. `pane.rename` — new pane → the Files label
6. `pane.send_input` — new pane → `yazi .` + `["enter"]`
7. `pane.split` — from the yazi pane, `direction = down`, `ratio = 0.9`,
   `cwd = <cwd>`, `focus = false` (**no** `Q_NO_BANNER` on this one)
8. `pane.rename` — new pane → the term label
9. `pane.send_input` — agent pane → the launch command + `["enter"]`
10. `tab.focus` — the tab

**Cancelling at any menu must create no Herdr resource at all.** The existing test
asserts an empty log on cancel. Once the tab exists, any subsequent failure must close
the tab and notify (`cleanup_tab` in `scripts/new-agent-popup.zsh:150-155`).

The in-pane launcher builds the same layout but from the other side: it renames the
pane and tab, runs every menu at the pane's **full width**, and defers the two splits
to the very end. That ordering is load-bearing — splitting earlier resizes the pane
mid-menu and breaks the centering, and a chosen worktree must be able to drive `cwd`
for all three panes.

## Restart-in-place

The mechanism, from `scripts/restart-agent.zsh`:

1. Resolve the target agent pane. If the focused pane has an `agent`, that is the
   target. Otherwise find the first pane in the same tab whose `agent` is non-null; if
   there is none, notify "No agent pane in this tab to restart." and exit 0.
2. If focus is elsewhere, walk `left right up down` with `pane.neighbor` from the
   focused pane until the neighbour is the target, then `pane.focus` in that
   direction. If no direction matches, notify "Could not focus the agent pane." and
   exit 1.
3. Read `pane.process_info`. If `foreground_process_group_id` is present, non-zero,
   and different from `shell_pid`: `kill -TERM -<pgid>`, poll up to 50 times at 100 ms,
   then `kill -KILL -<pgid>` if still alive, then sleep 300 ms so the shell settles
   back to its prompt.
4. Re-inject the launcher through `pane.send_input`, prefixed by a TTY reset:
   `stty sane; printf '\033[<u\033[?7h\033[?25h\033[0m'; <launcher …>`.
   Codex leaves the pane in raw mode with the Kitty keyboard protocol enabled when its
   process group is terminated — disabled ONLCR makes each newline continue at the old
   column, and Kitty CSI-u sequences make gum ignore arrow keys. The reset must run
   **inside** the pane, because the detached plugin process has no access to that TTY.
5. The re-injected launcher runs with no tab rename, the current label as a fixed
   usage (so the usage menu is skipped), no worktree step, and layout skipped.

**Why this works, and what must not change**: the launcher is injected via
`pane.send_input`, so it runs as a child of the pane's interactive shell. Its final
`exec` replaces the *launcher* process, not the shell. Killing the agent's foreground
process group therefore drops the pane back to its prompt instead of destroying it.
Rust must use `std::os::unix::process::CommandExt::exec()`, which is a real `execvp`
and never returns on success. Spawning the harness as a child process instead would
break this and is not acceptable.

## Registry schemas

Both registries are `{"version": 1, …}`, written atomically: write to a `mktemp`
sibling, pretty-print, then `rename` over the target. Never write the destination in
place.

### Project registry

Path: `$Q_PROJECT_REGISTRY_FILE`. Shape:

```json
{"version":1,"generated_at":"2026-07-30T12:00:00Z",
 "projects":{"/abs/path":{"name":"basename","sources":["claude","codex","filesystem","manual"],
   "aliases":["a","b"],"hidden":false,"last_used_at":1785474235}}}
```

- `generated_at` is UTC `%Y-%m-%dT%H:%M:%SZ`. `last_used_at` is a unix epoch integer.
- `sources` accumulate and are sorted-unique. `manual` is preserved across updates.
- Discovery merges three sources:
  - **Claude** — `~/.claude/projects`: `.entries[].projectPath` from every
    `sessions-index.json`, plus `cwd` values scraped from the `*.jsonl` transcripts.
  - **Codex** — `~/.codex/sessions`: the first line of every `rollout-*.jsonl`; take
    `payload.cwd` when `type == "session_meta"`, else `cwd`.
  - **Filesystem** — a `.git` sweep of `$Q_PROJECTS_ROOT`, pruning `node_modules`,
    `vendor`, `tmp`, `log`, `coverage`, `dist`, `build`, `.nuxt`, `.next`.
- **`canonical_project()` must keep its filter**: resolve to the git toplevel, resolve
  symlinks, reject `/`, and drop anything under `/tmp`, `/private/tmp`,
  `/var/folders/*/*/T`, or `/private/var/folders/*/*/T` **unless** it is inside the
  resolved `$Q_PROJECTS_ROOT`.
- `scan` refuses to run when the registry already exists; `rescan` requires it and
  marks rows `[new]` / `[missing]`; `update` refreshes sources with no prompt.
- Cancelling the review, or selecting nothing, must leave the registry unwritten.

### SSH registry

Path: `$Q_SSH_REGISTRY_FILE`. Shape:

```json
{"version":1,"targets":{"alias":{"source":"config","hostname":"h","user":"u",
  "aliases":["alias","alt"],"last_used_at":null,"hidden":false}}}
```

- `sync` reconciles against `$Q_SSH_CONFIG_FILE`. A `Host` line may declare several
  aliases; the registry is keyed by the **first** one. Resolve each with
  `ssh -G -F <config> -- <primary>` and read `hostname` and `user` from the output.
- Entries whose `source` is `config` but which are no longer in the config file are
  dropped. `manual` entries are left alone.
- On first run only — when the registry is absent or invalid — seed `manual` entries
  from `$Q_SSH_HISTORY_FILE`, most recent first, deduplicated.
- `remove` **hides** a config-sourced entry (`hidden = true`) and **deletes** a manual
  one.
- `use` stamps `last_used_at` and clears `hidden`. If the target is a bare
  `user@hostname` that matches exactly one config entry, the manual entry is deleted
  and the config entry is stamped instead.
- `list` emits NUL-delimited multi-line records for `fzf --read0`, sorted by
  `last_used_at` descending with never-used entries last, ordered by key. Config rows
  render as `aliases` / `user@hostname` / `[config]`; manual rows as the key then
  `[manual]`. The payload after a tab is the registry key.

## Picker wiring

Both pickers feed fzf NUL-delimited multi-line records with a tab-separated payload:
`--read0 --delimiter=$'\t' --with-nth=1 --accept-nth=2`.

Project picker (`scripts/project-picker-popup.zsh:20-26`):

- `--print-query --expect=alt-enter --prompt='Project> ' --highlight-line
  --pointer='▌' --info=inline-right --border --border-label-pos=bottom`
- border label: ` enter: agent · alt-enter: plain · ctrl-i: edit · typing searches zoxide `
- `--bind change:reload(<self> source {q})`
- `--bind ctrl-i:execute(<self> project edit {2})+reload(<self> source {q})`
- On a query that matches nothing registered, fall back to the directory itself if it
  exists, else `zoxide query -- <query>`.
- `enter` builds the agent tab in the new workspace; `alt-enter` leaves it plain. An
  existing workspace whose pane `cwd` or `foreground_cwd` matches is focused instead
  of creating a new one.

SSH picker (`scripts/ssh-picker-popup.zsh:10-16`):

- `--no-sort --print-query --prompt='SSH> ' --read0 --highlight-line --gap --gap-line
  --pointer='▌' --border --border-label-pos=bottom`
- border label: ` enter: connect · ctrl-i: edit · ctrl-x: remove `
- `--bind ctrl-i:execute(<self> ssh edit {2})+reload(<self> ssh list)`
- `--bind ctrl-x:execute-silent(<self> ssh remove {2})+reload(<self> ssh list)`

Any editor invoked from a binding must `clear` before drawing — fzf owns the alternate
screen and needs it clean to redraw when the command exits.

`<self>` is `std::env::current_exe()`, shell-quoted.

## Other behaviours that must survive

- **Popup cwd**: a plugin popup starts with the plugin directory as cwd, which is
  itself a git checkout — so a bare `git rev-parse` there resolves to the plugin, not
  the workspace. Adopt `focused_pane_cwd` from the plugin context before anything
  reads the working directory, falling back to `pane.get` on `HERDR_ACTIVE_PANE_ID`.
- **Worktree selection**: `git worktree prune` first, then offer only branches not
  already checked out in a worktree (git forbids the same branch twice). The field
  both filters existing branches and names a new one. An empty name becomes
  `wt-<unix timestamp>`. The directory is `<repo parent>/<repo name>-wt/<branch with
  slashes replaced by dashes>`. Reuse the directory if it exists; otherwise
  `git worktree add` with `-b` when the branch is new. On failure, fall back to no
  worktree rather than aborting.
- **Branch tagging**: when a worktree was chosen, the label becomes
  `<label>` + two spaces + `U+F169F`-free plain branch name — specifically
  `"$label  $branch"`.
- **Pane sizing** in the in-pane launcher comes from `pane.layout`, not `tput`: a
  restarted pane briefly reports the old size. Fall back to `COLUMNS`/`LINES`, then
  `tput`, in that order.
- **SSH session** owns one connection and closes its dedicated tab on exit, including
  on HUP/INT/TERM. On a clean exit it stamps `use` and appends a
  `: <epoch>:0;ssh <target>` line to `$HOME/.zsh_history`.
- **Dashboard** resolves its workspace by label every time (workspace ids are not
  durable), notifies and exits if the label is not found, creates a focused tab, and
  submits `claude --model sonnet <prompt>` where the prompt is
  `/usage-dashboard and restart /cockpit server`.

## Clause index

Every checkable requirement in this file, with a stable id. The final review writes one
row per id — that is what makes "every clause was walked" verifiable rather than a
matter of where a reader draws the boundaries. Ids never change; a new requirement gets
the next free number in its group.

| Id | Requirement |
|---|---|
| DEV-1 | In-pane launcher menu order is harness → model → usage |
| DEV-2 | Extra-args settings are string arrays; a file element may contain a space |
| DEV-3 | Every fatal path reports a concrete cause on its subcommand's channel |
| DEV-4 | Protocol guard: silent on match, one notification on mismatch |
| DEV-5 | Restart offers "use last combination" as the first harness entry |
| DEV-6 | The worktree is created after every menu, never before |
| GLY-1 | All 19 glyph literals and their two-space spacing match the codepoint table |
| GLY-2 | The model banner uses two spaces after `U+F09D1`, not one |
| GLY-3 | Menu selections are pad-stripped but keep their glyph |
| MSG-1 | The four notification titles and bodies match, at `bottom-right` |
| MSG-2 | The popup cleanup notification also passes `sound: "none"` |
| MSG-3 | Every stderr message and its exit code match the stderr table |
| MSG-4 | The three success lines go to stdout, not stderr |
| MSG-5 | Unnamed fatal paths use `<subcommand path>: <chained cause>`, exit 1 |
| MSG-6 | `project source` produces no error output at all |
| RST-1 | The confirm prompt uses the exact banner, labels and four colour flags |
| RST-2 | Target resolution: focused agent pane, else first agent pane in the tab |
| RST-3 | Focus walks `left right up down` before opening menus |
| RST-4 | Kill guard: pgid present, non-zero, different from the shell pid |
| RST-5 | TERM, 50 polls at 100 ms, KILL, 300 ms settle |
| RST-6 | The injected TTY-reset prefix is literal and unquoted |
| RST-7 | Re-injection: no tab id, current label as fixed usage, no worktree, no layout |
| RST-8 | The launcher ends in a real `execvp`; no wrapper process survives |
| CFG-1 | Every default resolves as the defaults table says |
| CFG-2 | Precedence is user file → environment → built-in default |
| CFG-3 | The model menu tables match, including per-label extra flags |
| CFG-4 | Both bypass flags stay opt-in, never defaulted |
| LAU-1 | Launch command assembly matches per harness, including `CCR` → `ccr code` |
| TAB-1 | The popup's ten-call sequence matches, in order, with every parameter |
| TAB-2 | `Q_NO_BANNER` on the tab and the first split, not the second |
| TAB-3 | Cancelling before the tab exists creates no Herdr resource |
| TAB-4 | Any failure after `tab.create` closes the tab |
| TAB-5 | The in-pane launcher defers both splits until after every menu |
| WRK-1 | Worktree selection: prune, exclude checked-out branches, auto-name on empty |
| WRK-2 | Directory is `<repo parent>/<repo name>-wt/<branch with slashes dashed>` |
| WRK-3 | A failed `git worktree add` falls back to no worktree, normalised |
| WRK-4 | A chosen worktree drives `cwd` for all three panes |
| LBL-1 | A branch suffix is appended to the label as `<label>  <branch>` |
| SIZ-1 | In-pane sizing reads `pane.layout`, then `COLUMNS`/`LINES`, then `tput` |
| CWD-1 | The popup adopts `focused_pane_cwd`, then `HERDR_ACTIVE_PANE_ID` |
| RGP-1 | Project registry schema, atomic write, and two-space + newline formatting |
| RGP-2 | `canonical_project` filter, including the projects-root exception |
| RGP-3 | Three discovery sources and their prune list |
| RGP-4 | `sources` accumulate, sorted-unique, `manual` preserved |
| RGP-5 | `scan`/`rescan`/`update`/`use`/`edit` guards and markers |
| RGP-6 | Cancelling or selecting nothing leaves the registry unwritten |
| RGS-1 | SSH registry schema and atomic write |
| RGS-2 | Config reconciliation drops stale config entries, keeps manual ones |
| RGS-3 | History seeding happens only on an absent or invalid registry |
| RGS-4 | `remove` hides config entries and deletes manual ones |
| RGS-5 | `use` resolves aliases and collapses a matching `user@hostname` entry |
| RGS-6 | `list` record format, NUL separation, two-space alias join, sort order |
| PIK-1 | Project picker fzf flags, prompt, border label and three bindings |
| PIK-2 | Project picker fallbacks: directory, then zoxide |
| PIK-3 | Existing workspace is focused only; new workspace branches on the key |
| PIK-4 | Project source record format and sort order |
| PIK-5 | SSH picker fzf flags, prompt, border label and two bindings |
| PIK-6 | A binding-invoked editor clears the screen before drawing |
| SSH-1 | `ssh edit` validation rules and the duplicate-alias refusal |
| SSH-2 | The appended config block layout, written atomically |
| SSH-3 | The session closes its tab on every exit path |
| SSH-4 | A clean exit stamps `use` and appends the history line |
| DSH-1 | The dashboard resolves its workspace by label every run and creates none |
| DSH-2 | The dashboard tab is focused, and the prompt is submitted as one argument |
