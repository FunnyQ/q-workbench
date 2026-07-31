# REGISTRY-05: SSH registry store, reconciliation and operations

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: registry/04
> **Blocks**: picker/03, picker/04, polish/05
> **Status**: done

## Goal

`workbench ssh {sync|list|get|use|remove}` producing a registry JSON file and a
NUL-delimited `list` output byte-identical to the zsh version's.

## Files to create / modify

- `src/registry/ssh.rs` (modify) — the store, reconciliation, and the five operations
- `src/main.rs` (modify) — wire the five subcommands

## Implementation notes

The registry path, schema, reconciliation rules, seeding behaviour, hide-versus-delete
semantics, and the `list` record format are all inlined in the parity contract. The two
input sources — config records and history targets — already exist and are consumed
here.

### Replacing the jq reduce

`scripts/ssh-target-registry.zsh:66-78` reconciles the registry against the SSH config
in one jq `reduce`. Rewrite it as plain Rust over a `BTreeMap<String, SshTarget>`: drop
config-sourced entries no longer present in the config, then upsert every configured
record. `BTreeMap` for stable key order, as with the project registry.

Keep reconciliation a **pure function** over `(existing registry, config records) ->
registry`, so it tests without touching the filesystem.

### Seeding

Seeding from history happens **only** when the registry file is absent or fails
validation — never on a normal sync. Getting this wrong re-adds every host the user has
deliberately removed, every time they sync.

### `list` output

The record format, sort order and field layout are in the parity contract. Two details
that are easy to lose:

- Records are separated by NUL bytes, not newlines — the records themselves are
  multi-line. The zsh version emitted `\f` from jq and translated it with `tr`; write
  the NUL directly.
- Config rows join their aliases with **two** spaces.

The sort is: entries with a `last_used_at` first, most recent first; then entries
without one, ordered by key. Note this orders by **key**, unlike the project picker
source, which orders its never-used entries by display name.

### `use` and alias resolution

A config `Host` may declare several aliases; the registry is keyed by the first. `use`
resolves an alias to its key before stamping. It also collapses a bare `user@hostname`
manual entry into a config entry when exactly one config entry matches that hostname
and user — deleting the manual entry. Reproduce that exactly; it is what keeps the
registry from accumulating duplicates after a host is added to the config.

### Atomic write

Same as the project registry: `mktemp` sibling, pretty-print with two-space indentation
and a trailing newline, then rename over the target. `trash` the temp file on failure.
Never truncate the destination first.

There is no `generated_at` field here, so the JSON is fully deterministic and the parity
comparison needs no normalisation.

## Acceptance criteria

- [x] `sync` drops config-sourced entries no longer in the config and leaves manual
      entries alone.
- [x] Seeding happens only when the registry is absent or invalid.
- [x] `remove` hides a config-sourced entry and deletes a manual one.
- [x] `use` stamps `last_used_at`, clears `hidden`, resolves aliases, and collapses a
      matching `user@hostname` manual entry into its config entry.
- [x] `list` emits NUL-delimited multi-line records in the documented sort order, with
      config aliases joined by two spaces.
- [x] Reconciliation is a pure function that touches no filesystem.
- [x] Writes are atomic, two-space indented, with a trailing newline.

## Verification

- [x] `cargo test` — reconciliation over a fixture registry and fixture config records:
      entry removed from config, entry added, manual entry untouched
- [x] `cargo test` — seeding runs for an absent registry and for an invalid one, and
      does **not** run for a valid one
- [x] `cargo test` — `use` collapsing a `user@hostname` manual entry into a config entry,
      and leaving it alone when two config entries match
- [x] `cargo test` — `remove` hides a config entry and deletes a manual one
- [x] **Byte-identical check**: run the zsh `scripts/ssh-target-registry.zsh sync` and
      the Rust `ssh sync` against the same fixture config and registry copy, then `diff`
      the JSON and `cmp` the `list` output — the latter contains NUL bytes, so `diff`
      is not enough
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Deletes what should be hidden, or seeds on every sync | Reconciliation right but sort order or alias joining differs | JSON and `list` both byte-identical; every semantic reproduced |
| Test coverage | ×2 | No fixtures | Reconciliation only | Reconciliation, all three seeding cases, alias collapse, remove, and both byte-identical comparisons |
| Interface & readability | ×1 | jq reduce transcribed into Rust | Readable but reconciliation takes I/O | Reconciliation pure over maps; I/O confined to the store |
| Assumptions & docs | ×1 | No note on why seeding is one-shot | Mentioned briefly | Seeding, hide-vs-delete, and the alias-collapse rule all explained |

## Out of scope

- Parsing the SSH config and the history file — both already exist and are consumed here.
- The picker, the host editor, and the session wrapper.
