# Herdr socket API

> Everything below was measured against `herdr 0.7.5`, protocol 17, on this machine.
> The full 248 KB JSON Schema sits beside this file as
> `herdr-api-schema-protocol17.json`;
> regenerate with `herdr api schema --json`. You do not need to open it for the common
> methods — the shapes you need are inlined here.

## Transport

- **Address**: the `HERDR_SOCKET_PATH` environment variable, already present in every
  pane and plugin process. It resolves to `~/.config/herdr/herdr.sock`.
- **Protocol**: newline-delimited JSON over a Unix domain socket.
- **One request per connection.** The server closes the connection immediately after
  writing its single response. Verified: a second `ping` sent 150 ms later on the same
  socket gets no reply. There is no multiplexing, no keep-alive, no request pipelining.
  Connect, write one line, read to the newline, close.
- **Responses arrive in multiple chunks.** `pane.list` returns ~12.9 KB split across
  several reads. A client that assumes one chunk works intermittently — the worst kind
  of bug. Always accumulate until a `\n` is seen.
- Connect + round-trip costs 2.1–2.5 ms, versus ~9.8 ms for the equivalent `herdr` CLI
  invocation.

## Envelope

Request — both `method` and `params` are required; `params` may be `{}`:

```json
{"id":"1","method":"ping","params":{}}
```

Success:

```json
{"id":"1","result":{"type":"pong","version":"0.7.5","protocol":17,
  "capabilities":{"live_handoff":true,"detached_server_daemon":true}}}
```

Error:

```json
{"id":"1","error":{"code":"invalid_key","message":"unsupported key cr"}}
```

Note the error `id` is `""` rather than the request id when the request itself failed
to deserialise. Do not key error handling on the id.

## Methods this plugin uses

There are 89 methods across `server.*`, `client.*`, `session.*`, `workspace.*`,
`worktree.*`, `tab.*`, `agent.*`, `pane.*`, `layout.*`, `popup.*`, `events.*`,
`integration.*`, `plugin.*`, and `notification.show`. This plugin uses these:

| Old CLI call | Socket method | Params |
|---|---|---|
| `tab create` | `tab.create` | `{workspace_id, label, cwd, env, focus}` |
| `tab rename` / `focus` / `close` | `tab.rename` / `tab.focus` / `tab.close` | `{tab_id, …}` |
| `pane split` | `pane.split` | `{target_pane_id, direction, ratio, cwd, env, focus}` — `direction` required, enum `right`\|`down` |
| `pane run <pane> "<cmd>"` | `pane.send_input` | `{pane_id, text, keys}` |
| `pane rename` | `pane.rename` | `{pane_id, label}` |
| `pane list` / `current` / `get` / `layout` / `process-info` / `neighbor` / `focus` | same-named `pane.*` methods | |
| `workspace create` / `list` / `focus` | `workspace.create` / `.list` / `.focus` | `create` takes `{cwd, env, focus, label}` |
| `api snapshot` | `session.snapshot` | `{}` |
| `notification show` | `notification.show` | `{title, body, position, sound}` |
| `plugin config-dir q.workbench` | **no equivalent** | use the literal config path instead |

The `plugin config-dir` gap is harmless: the config path is a documented literal,
`${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench`.

## `pane.send_input` — the replacement for `herdr pane run`

```json
{"method":"pane.send_input",
 "params":{"pane_id":"w2N:p4","text":"echo hello","keys":["enter"]}}
```

- `pane_id` is the only required field.
- `text` is typed into the pane literally. **The pane's interactive shell then
  interprets it**, so anything embedded in it must be shell-quoted by the caller. The
  zsh version relied on `${(q)}` for this; the Rust version needs its own
  `shell_quote()`. This is the one genuinely new failure surface in the rewrite.
- `keys` accepts `enter`, `Enter`, and `return` (verified working). `cr` is rejected
  with `{"code":"invalid_key","message":"unsupported key cr"}`. The schema declares
  only `array of string` with no enum, so the accepted vocabulary is not discoverable
  from the schema — use `enter`.
- Sending `text` and `keys` in one call is exactly equivalent to `herdr pane run`:
  the text is typed and then submitted.

## Response shapes you will need

`tab.create`:

```json
{"result":{"type":"tab_created",
  "root_pane":{"pane_id":"w2N:p4","tab_id":"w2N:t2","workspace_id":"w2N",
    "cwd":"/private/tmp","focused":false,"agent_status":"unknown",
    "foreground_cwd":"/private/tmp","terminal_id":"…","revision":0},
  "tab":{"tab_id":"w2N:t2","workspace_id":"w2N","label":"scratch","number":2,
    "pane_count":1,"focused":false,"agent_status":"unknown"}}}
```

`workspace.create` returns `type: "workspace_created"` with three required objects —
`workspace`, `tab`, and `root_pane` — so one call yields everything needed to rename
the tab and inject into its pane:

```json
{"result":{"type":"workspace_created",
  "workspace":{"workspace_id":"w2P","number":3,"label":"my-project","focused":false,
    "pane_count":1,"tab_count":1,"active_tab_id":"w2P:t1","agent_status":"unknown",
    "worktree":null,"tokens":{}},
  "tab":{"tab_id":"w2P:t1","workspace_id":"w2P","label":"…","number":1,
    "pane_count":1,"focused":false,"agent_status":"unknown"},
  "root_pane":{"pane_id":"w2P:p1","tab_id":"w2P:t1","workspace_id":"w2P","cwd":"…",
    "foreground_cwd":"…","focused":false,"agent_status":"unknown",
    "terminal_id":"…","revision":0}}}
```

The fields callers need are `workspace.workspace_id`, `tab.tab_id` and
`root_pane.pane_id`. `workspace.worktree` is null unless the workspace is a git
worktree. Authoritative source, for verification only:
`herdr-api-schema-protocol17.json` in this directory, `ResponseResult` variant
`workspace_created`.

`pane.split` returns `{"result":{"pane":{"pane_id":…}}}`.

`pane.list` returns `{"result":{"panes":[…]}}`, each pane carrying `pane_id`,
`terminal_id`, `workspace_id`, `tab_id`, `focused`, `cwd`, `foreground_cwd`, `label`,
`agent`, `terminal_title`, `terminal_title_stripped`, plus rect and revision.
**Client-side filtering of `pane.list` is the search** — that is what
`scripts/project-picker-popup.zsh:50-55` does today via a snapshot piped through `jq`.

`pane.process_info` carries `foreground_process_group_id` and `shell_pid` — both are
what restart-in-place kills and compares against.

`ping` returns `{"type":"pong","version","protocol","capabilities"}`.

Simple mutations return `{"result":{"type":"ok"}}`.

## Plugin invocation context

When Herdr invokes a plugin action or opens a plugin pane, the process receives a
context describing where it was invoked from. Over the socket the same shape appears
in `plugin.action.invoke` results:

```json
{"workspace_id":"w2N","workspace_label":"herdr-workbench",
 "workspace_cwd":"/Users/funnyq/Projects/q-lab/herdr-workbench",
 "tab_id":"w2N:t1","tab_label":"main","focused_pane_id":"w2N:p1",
 "focused_pane_cwd":"/Users/funnyq/Projects/q-lab/herdr-workbench",
 "focused_pane_agent":"claude","focused_pane_status":"working",
 "invocation_source":"api","correlation_id":"1"}
```

In the process environment this arrives as `HERDR_PLUGIN_CONTEXT_JSON`, alongside
`HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_ACTIVE_PANE_ID` and
`HERDR_ENV`. A popup's own cwd is the **plugin directory**, not the invoking pane's —
so any flow that reads `$PWD` must first adopt `focused_pane_cwd`.

## Plugin manifest facts

- `command[0]` accepts a **relative executable** — verified for both an `[[actions]]`
  and a `[[panes]]` entry. The process starts with the plugin root as its cwd, and
  `argv[0]` arrives as written (`./bin/probe`, or `<root>/./bin/probe` for panes).
- `herdr plugin install <owner>/<repo>` is a real `git clone`. The installed tree at
  `~/.config/herdr/plugins/github/q.workbench-<hash>/` contains a `.git` directory.
  Nothing is built at install time.

## Not used, but verified

`events.subscribe` holds a long-lived connection: it replies
`{"type":"subscription_started"}` and then streams event lines indefinitely. Its
`subscriptions` parameter is an **internally-tagged enum**, not a list of strings —
`[{"type":"pane.focused"}]`, not `["pane"]`. No current feature needs it.
