# FOUNDATION-02: Herdr socket client

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
>
> **Depends on**: foundation/01
> **Blocks**: foundation/05, picker/02, picker/03, agent/02, agent/03, agent/04, agent/05, polish/01
> **Status**: todo

## Goal

A blocking Unix-socket client covering the methods this plugin needs, behind a
`HerdrClient` trait, with a recording fake for tests and a real-socket integration
test proving the wire handling.

## Files to create / modify

- `src/herdr/mod.rs` (new) — `HerdrClient` trait, `SocketClient`, `FakeClient`
- `src/herdr/types.rs` (new) — serde structs for requests and responses
- `src/main.rs` (modify) — implement `workbench herdr ping`
- `tests/socket_client.rs` (new) — integration test against a real `UnixListener`

## Implementation notes

### The two wire gotchas

Both were found by probing the live server, and both produce intermittent bugs if
missed:

1. **One request per connection.** The server closes the connection right after
   writing its single response. Connect, write one line, read to the newline, drop the
   stream. Never reuse a connection; a client that does will hang.
2. **Responses arrive in multiple chunks.** Accumulate into a buffer until a `\n`
   appears, then parse the first line. A ~12.9 KB `pane.list` response reliably
   arrives split.

### Shape

Keep it small and owned. Something like:

```rust
pub trait HerdrClient {
    fn call(&self, method: &str, params: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}
```

`call` returns the **`result` object**, and turns an `error` object into an
`anyhow::Error` carrying the code and message. Typed helpers sit on top as free
functions or default trait methods — `tab_create`, `pane_split`, `pane_send_input`,
`pane_rename`, `pane_list`, `pane_get`, `pane_current`, `pane_layout`,
`pane_process_info`, `pane_neighbor`, `pane_focus`, `tab_rename`, `tab_focus`,
`tab_close`, `workspace_create`, `workspace_list`, `workspace_focus`,
`session_snapshot`, `notification_show`, `ping`. Each deserialises into a struct from
`types.rs`.

`SocketClient::new()` reads `HERDR_SOCKET_PATH` and errors with a clear message when
it is unset — that variable is present in every pane and plugin process, so its
absence means the binary is being run outside Herdr.

Request ids can be a monotonically increasing counter or a constant `"1"`; there is no
multiplexing, so it does not matter. Do not key error handling on the returned id — a
request that fails to deserialise comes back with `id: ""`.

### `FakeClient`

The zsh tests worked by putting a fake `herdr` executable on `PATH` and asserting on
the log it wrote. `FakeClient` replaces that: it records every `(method, params)` in
order and returns canned results, so a test can assert the exact call sequence the
parity contract specifies.

```rust
pub struct FakeClient {
    pub calls: RefCell<Vec<(String, serde_json::Value)>>,
    /// Per-method FIFO of canned responses, consumed in call order.
    pub responses: RefCell<HashMap<String, VecDeque<serde_json::Value>>>,
}
```

**A map from method to a single response is not enough**: the layout flow calls
`pane.split` twice and each call must return a different pane id. Queue responses per
method and pop from the front on each call.

Exhaustion rules, so a test failure is legible rather than mysterious:

- Queue empty, or no queue for that method → return `{"type":"ok"}`. Most mutations
  return exactly that, so the common case needs no setup at all.
- Provide a way to queue an **error** response too, since several tasks inject a failure
  at a chosen call.
- Expose the recorded calls so a test can assert the exact sequence and every parameter.

### Types

Write `serde` structs by hand for the methods listed above rather than generating all
89 from the schema. Derive `Deserialize` with `#[serde(default)]` on optional fields —
several pane fields are absent rather than null in some responses. Response shapes for
the common methods are inlined in the API context file.

### `workbench herdr ping`

Prints the version and protocol, e.g. `herdr 0.7.5, protocol 17`. This is the
diagnostic the protocol guard later builds on, and the quickest smoke test that the
socket layer works at all.

## Acceptance criteria

- [ ] `HerdrClient` is a trait; `SocketClient` and `FakeClient` both implement it.
- [ ] `SocketClient` opens one connection per call and closes it after reading.
- [ ] Responses split across multiple reads are reassembled correctly.
- [ ] An `error` response becomes an `anyhow::Error` carrying both code and message.
- [ ] A missing `HERDR_SOCKET_PATH` produces a clear error, not a panic.
- [ ] `FakeClient` records calls in order, and holds a per-method FIFO so two calls to
      the same method return different responses.
- [ ] An empty or absent queue yields `{"type":"ok"}`; an error response can be queued.
- [ ] `workbench herdr ping` prints the version and protocol number.

## Verification

- [ ] `cargo test` — integration test spins a real `UnixListener` in a tempdir, writes
      a response **deliberately split across two writes with a delay**, and asserts the
      client reassembles it
- [ ] Integration test asserts that the client does not attempt a second request on
      the same connection
- [ ] Integration test asserts an `error` response becomes an `Err` with the code in
      the message
- [ ] `./bin/workbench herdr ping` against the live Herdr prints `protocol 17`
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Reuses connections, or assumes single-chunk responses | Works against the live server but the chunking path is untested | One connection per call, buffers to newline, errors surfaced with code and message |
| Test coverage | ×2 | No socket test | Happy path only | Split-response, error-response, and missing-env-var paths all covered |
| Interface & readability | ×1 | Callers build raw JSON everywhere | Trait exists but typed helpers are inconsistent | Small trait, typed helpers, `FakeClient` easy to assert against |
| Assumptions & docs | ×1 | No comment on why connections are not reused | Mentions it in passing | Both wire gotchas commented at the point they are handled |

## Out of scope

- Startup protocol assertion — a later task wires the guard and its notification.
- `events.subscribe`. Verified to work but nothing needs it.
- Generating types for all 89 methods.
