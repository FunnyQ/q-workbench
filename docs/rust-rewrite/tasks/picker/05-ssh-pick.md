# PICKER-05: `workbench ssh pick`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: picker/03, picker/04, foundation/05
> **Blocks**: polish/03
> **Status**: todo

## Goal

The SSH popup: fuzzy-pick a host and connect to it in a dedicated tab.

## Files to create / modify

- `src/flows/picker.rs` (modify) — add the SSH picker
- `src/main.rs` (modify) — wire `ssh pick`

## Implementation notes

The fzf argument list, bindings and border label are in the parity contract.

### Selection

Feed fzf from the registry's `list` output. With `--print-query`, fzf writes the query
then the selection; use the selection, falling back to the raw query so an unlisted
host can still be typed and connected to. A non-zero fzf exit means cancelled — exit 0
with no side effect.

### Bindings

`ctrl-i` runs the host editor and reloads; `ctrl-x` removes silently and reloads. Both
call this same binary, so build them from `std::env::current_exe()`, shell-quoted,
never a hardcoded path — the location differs between the installed plugin and the dev
harness.

### Connect

Create the tab with the parity contract's label, `Q_NO_BANNER` set, focus off; send
the session command into its root pane; then focus the tab. Any failure after
`tab.create` closes the tab.

The session command embeds the target and the tab id, both from outside this program.
Quote all three parts — the session subcommand path, the target, and the tab id —
through the shell helper. The zsh version applied `${(q)}` to all three for exactly
this reason.

## Acceptance criteria

- [ ] The fzf invocation matches the parity contract, including `--no-sort`, `--gap`,
      `--gap-line`, the prompt, the pointer, the border label and both bindings.
- [ ] Bindings are built from the running executable's own path, shell-quoted.
- [ ] Cancelling fzf exits 0 with no side effect.
- [ ] A typed but unlisted target still connects.
- [ ] The tab is created with the parity contract's label, `Q_NO_BANNER`, and focus off,
      then focused after the session command is sent.
- [ ] All three parts of the session command are shell-quoted.
- [ ] Any failure after `tab.create` closes the tab.

## Verification

- [ ] `cargo test` — result parsing for selection present, selection absent with a
      query, and both absent
- [ ] `cargo test` — binding strings contain the current executable path, correctly
      quoted for a path containing a space
- [ ] `cargo test` — with `FakeClient`, the success sequence is `tab.create`,
      `pane.send_input`, `tab.focus`; a failure injected at either of the last two
      issues `tab.close`
- [ ] `cargo test` — the session command text round-trips through a shell back to the
      intended argv, including a target containing a character needing quoting
- [ ] Manual through the linked dev plugin: connect to a real host and confirm the tab
      label and that the connection works
- [ ] Manual: `ctrl-x` on a config host hides it, on a manual host deletes it, and the
      list reloads cleanly in both cases
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Hardcodes the binary path, or a failure leaves the tab open | fzf flags right but the fallback-to-query path or the quoting differs | Every flag, binding and branch reproduced; all three parts quoted |
| Test coverage | ×2 | No parsing tests | Parsing only | Parsing, binding construction, success and both failure injections, quoting round-trip |
| Interface & readability | ×1 | fzf argument list built inline as one string | Extracted but hard to compare against the original | Arguments listed one per line, directly comparable to the parity contract |
| Assumptions & docs | ×1 | No note on why all three parts are quoted | Mentioned briefly | Explains that the pane's shell interprets the text, so every embedded value is quoted |

## Out of scope

- The session lifecycle and the host editor — both exist already; this task only wires
  the picker to them.
