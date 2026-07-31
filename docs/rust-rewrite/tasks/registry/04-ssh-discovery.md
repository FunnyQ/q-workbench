# REGISTRY-04: SSH config and history discovery

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/03
> **Blocks**: registry/05
> **Status**: todo

## Goal

The two input sources for the SSH registry — the SSH config file and the shell history
— parsed into plain data, with no registry file involved.

## Files to create / modify

- `src/registry/ssh.rs` (new) — config parsing, `ssh -G` resolution, history parsing

## Implementation notes

This task produces no CLI subcommand and touches no registry file. Like the project
registry's discovery layer, it exists on its own because these are the parsing rules
most likely to drift silently and the easiest to test in isolation.

### Alias groups from the config

Only the `Host` lines are read directly. Match the first field case-insensitively
against `host`, then take every subsequent field containing none of `*`, `!`, `?`. A
group with no surviving alias is skipped. Groups are deduplicated and sorted.

The wildcard exclusion matters: `Host *` and `Host !prod` are pattern rules, not
connectable targets, and putting them in the registry would fill the picker with
entries that cannot be used.

### Resolving each group

Everything else comes from `ssh -G -F <config> -- <primary alias>`, which resolves the
effective configuration including anything inherited from a `Host *` block. Read the
**first** `hostname` line and the **first** `user` line from its output; `ssh -G` can
emit a key more than once and the first wins.

A non-zero exit means that alias cannot be resolved — skip it and continue, rather than
failing the whole sync. A broken entry in a long config must not take the rest with it.

Put the `ssh -G` call behind a small injectable runner — a function parameter or a
one-method trait — so the parsing above can be tested without a real `ssh` on `PATH`.
`history_targets` needs no such thing; it is pure over the file contents.

```rust
pub struct ConfigRecord {
    pub target: String,        // the first alias — the registry key
    pub hostname: String,
    pub user: String,
    pub aliases: Vec<String>,  // every alias in the group, in file order
}

pub fn config_records(config: &Path) -> Vec<ConfigRecord>
```

### History parsing

The zsh version matched extended-history lines of the form
`: <epoch>:<elapsed>;[TERM=… ]ssh <args> <target>`, stripped the flags between `ssh`
and the target, kept the most recent occurrence of each target, and accepted only
targets matching `[][a-zA-Z0-9_.@-]+`. Reproduce all of it, including that character
class — the brackets are literal `[` and `]`, which appear in bracketed IPv6 targets.

Order the result most recent first. A missing history file yields an empty list.

```rust
pub fn history_targets(history: &Path) -> Vec<String>
```

## Acceptance criteria

- [ ] `Host` lines are matched case-insensitively; aliases containing `*`, `!` or `?`
      are excluded, and a group left with none is skipped.
- [ ] Groups are deduplicated and sorted; the first alias is the key and the full group
      is kept in file order.
- [ ] `hostname` and `user` come from the first matching line of `ssh -G` output.
- [ ] The `ssh -G` invocation sits behind an injectable runner, so config parsing is
      testable without a real `ssh` binary.
- [ ] An alias `ssh -G` cannot resolve is skipped, not fatal.
- [ ] History parsing handles the `TERM=…` prefix, flags before the target, and
      duplicates, keeping the most recent and rejecting targets outside the character
      class.
- [ ] A missing config or history file yields an empty list, not an error.

## Verification

- [ ] `cargo test` — a fixture SSH config with a `Host *` block, a multi-alias group, a
      `Host !prod` negation, and a group of only wildcards; assert exactly which groups
      survive and in what order
- [ ] `cargo test` — `ssh -G` output containing a repeated `hostname` key yields the
      first value
- [ ] `cargo test` — with a stubbed runner (no real `ssh` involved), an alias whose
      resolution exits non-zero is skipped and the others survive
- [ ] `cargo test` — a fixture history file with a `TERM=…` prefix, `ssh -p 22 host`,
      duplicates at different timestamps, and one target containing a space; assert the
      order and the rejections
- [ ] `cargo test` — both functions return empty for a missing file
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Wildcard aliases leak in, or the history character class differs | Parsing right but a repeated `ssh -G` key or a skip case is wrong | Every parsing rule, ordering rule and skip case reproduced |
| Test coverage | ×2 | No fixtures | Config only | Config, `ssh -G` edge cases, history, and both missing-file cases |
| Interface & readability | ×1 | Parsing entangled with registry state | Registry-independent, but `ssh -G` is spawned inline so the parsing cannot be tested without it | History parsing pure; config parsing registry-independent with the `ssh -G` call behind an injectable runner, so both are testable without a real `ssh` |
| Assumptions & docs | ×1 | Wildcard exclusion uncommented | Mentioned without the reason | Explains why patterns are excluded and why a failed `ssh -G` is survivable |

## Out of scope

- The registry file, reconciliation, and the CLI operations — the next task.
