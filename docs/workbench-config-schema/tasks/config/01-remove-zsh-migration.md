# CONFIG-01: Remove the zsh migration surface

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: none — foundation task
> **Blocks**: config/02
> **Status**: done

## Goal

`workbench config migrate` and every function, test, and fixture behind it are gone, leaving `src/config.rs` small enough that the schema rewrite lands on a clean file.

## Files to create / modify

- `src/config.rs` (modify) — delete the migration half of the file; keep loading, defaults, and validation.
- `src/main.rs` (modify) — delete the `config` command tree and the two helpers only it used.
- `tests/fixtures/config.fixture` (delete) — the zsh fixture the migration tests read.

## Implementation notes

The plugin migrated from zsh to Rust one release ago. `workbench config migrate` was the bridge: it executed a `config.zsh` and emitted equivalent TOML. That bridge has been crossed. The schema this file is about to grow has no zsh ancestor, so teaching the migration to emit it would be work for a user base of zero.

Delete first, rewrite second. Doing it in this order keeps the later schema work from having to reason about `PartialConfig`'s parallel field list.

### Delete from `src/config.rs`

Types and constants:

- `PartialConfig` — the whole struct and its `Serialize` derive.
- `SCALAR_SETTINGS` — the shell-name → TOML-name table.
- `SOURCE_FAILED_STATUS`.

Functions:

- `migrate()`
- `serialize_migration()`
- `push_model_table()`
- `resolved_config_path()`
- `migration_source_path()`
- `legacy_config_path()`
- `parse_dump()`
- `take_dump_string()`
- `take_record_values()`
- `dump_string()`
- `set_scalar()`

Tests in `mod tests`:

- `fixture_migrates_all_set_values_and_omits_absent_values`
- `missing_migration_source_is_a_clear_error`
- `a_source_zsh_cannot_parse_is_an_error_not_an_empty_config`
- `model_labels_that_look_like_assignments_survive_serialization`
- `the_extra_args_note_appears_only_when_an_extra_args_setting_is_migrated`
- `the_real_config_zsh_round_trips_when_it_exists`
- `default_migration_source_uses_xdg_config_home`

`use std::process::Command;` and the `Serialize` half of `use serde::{Deserialize, Serialize};` become unused once `migrate()` and `PartialConfig` are gone. `clippy` will name anything else.

### Keep the zsh-extension guard

`Config::load()` opens with a check that the resolved config path ends in `.zsh`:

```rust
if path.extension().is_some_and(|extension| extension == "zsh") {
    bail!(
        "config file {} is zsh, not TOML. Run `workbench config migrate --write`, \
         then unset Q_WORKBENCH_LOCAL_CONFIG or point it at the config.toml",
        path.display()
    );
}
```

The guard stays — a shell started before the cutover still exports `Q_WORKBENCH_LOCAL_CONFIG` pointing at `config.zsh`, and without this the user sees a TOML syntax error on `typeset -gA`. But the message advertises a command that will no longer exist. Rewrite it so the remedy is the one that still works:

```rust
bail!(
    "config file {} is zsh, not TOML. Unset Q_WORKBENCH_LOCAL_CONFIG, \
     or point it at a config.toml",
    path.display()
);
```

Its test `a_zsh_override_names_the_migration_instead_of_failing_to_parse_toml` currently asserts three substrings: `"is zsh, not TOML"`, `"workbench config migrate --write"`, and `"Q_WORKBENCH_LOCAL_CONFIG"`. Drop the middle assertion and add one for `"config.toml"`. Rename the test to something that describes what it now guards, for example `a_zsh_override_names_the_real_problem_instead_of_failing_to_parse_toml`.

### Delete from `src/main.rs`

`ConfigCommand::Migrate` is the only variant of `ConfigCommand`, so the whole command tree goes, not just the arm:

- The `Config { command: ConfigCommand }` variant of `enum Command` (around line 45).
- `enum ConfigCommand` and its long doc comment (around line 125).
- The `Command::Config { command: ConfigCommand::Migrate { .. } }` arm in `channel()` (around line 213). It sits in an or-pattern chain ending in `=> Channel::Stderr { uses_herdr: false }`; remove just that alternative and leave the chain intact.
- `Command::Config { .. } => "config migrate",` in `subcommand_path()` (around line 254).
- The whole `Command::Config { command } => match command { ... }` execution arm in `run()` (around line 427).
- `guard_write_destination()` (around line 525) and `write_atomically()` (around line 545). Both exist only for `--write`; confirm with a search before deleting, and if either has another caller, keep it.

Test call sites in `src/main.rs`:

- `every_leaf_parses_with_all_supported_arguments` — remove the `["workbench", "config", "migrate", "--from", "/tmp/config", "--write", "--force"]` case.
- `every_subcommand_selects_its_fixed_channel` — remove the `["workbench", "config", "migrate"]` case.
- `stderr_preserves_contract_messages_and_prefixes_unnamed_failures` — its loop iterates `["ssh sync", "config migrate", "herdr ping"]`; drop the middle entry.
- `terminal_subcommands_never_call_notification_show` — remove the `["workbench", "config", "migrate"]` case.

Removing the `Config` variant may leave `use std::fs;`, `Path`, or `bail!` unused in `main.rs`. Let `clippy` decide.

### Delete the fixture

```zsh
trash tests/fixtures/config.fixture
```

Use `trash`, not `rm`. If `tests/fixtures/` is then empty, leave the directory — later work may use it.

## Acceptance criteria

- [x] `src/config.rs` contains no `migrate`, `PartialConfig`, `SCALAR_SETTINGS`, or NUL-dump parsing code.
- [x] `src/main.rs` has no `Config` variant, no `ConfigCommand` enum, and no `guard_write_destination` / `write_atomically`.
- [x] `workbench config migrate` is no longer a parseable subcommand — `Cli::try_parse_from(["workbench", "config", "migrate"])` returns an error.
- [x] The zsh-extension guard in `Config::load()` still fires, and its message names `Q_WORKBENCH_LOCAL_CONFIG` and `config.toml` but not a `config migrate` command.
- [x] `tests/fixtures/config.fixture` is gone.
- [x] No behaviour outside the migration path changed: `dashboard_workspace`, the path scalars, `order`, `models`, and `model_args` all still load exactly as before.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean — this is the real check for leftover imports and dead helpers.
- [x] `rg 'config migrate|PartialConfig|serialize_migration|SCALAR_SETTINGS' src/` returns no matches.
- [x] `rg 'config.fixture' .` returns no matches outside `docs/`.
- [x] Run `git status --short` and quote it. Expect `src/config.rs`, `src/main.rs`, the deleted `tests/fixtures/config.fixture`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Deletes the zsh-extension guard, or breaks loading of a setting unrelated to migration | Migration code gone but the guard's message still advertises `config migrate`, or a `main.rs` test case was missed | Every listed symbol gone, guard kept with a corrected message, nothing else changed |
| Test coverage | ×2 | Deletes tests without checking the remaining suite still covers loading | Suite passes but the reworded guard has no assertion on its new text | The guard test asserts the new message, and every migration-only test is removed rather than commented out |
| Interface & readability | ×1 | Leaves `#[allow(dead_code)]` or commented-out blocks behind | Dead imports left for clippy to complain about | File reads as if the migration never existed; no orphan imports, no stubs |
| Assumptions & docs | ×1 | Silently changes an unrelated error message | Removes `guard_write_destination` without confirming it had no other caller | Confirms each helper's callers before deleting, and the stale `CLAUDE.md` paragraph is reported, not silently edited |

## Out of scope

- **`CLAUDE.md`** — it documents `workbench config migrate` as "the only compatibility boundary with the old zsh config". That paragraph is stale the moment this task lands, but documentation is rewritten once, later, alongside the CHANGELOG entry. Do not edit it here.
- **`CHANGELOG.md`** — the breaking-change entry covers this deletion together with the schema rewrite. Deferred to the documentation task in the wiring bucket.
- **Rebuilding `bin/workbench`** — the committed binary is rebuilt once, at the end of the plan.
