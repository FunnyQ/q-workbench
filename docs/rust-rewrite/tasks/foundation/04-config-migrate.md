# FOUNDATION-04: `workbench config migrate`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/03
> **Blocks**: cutover/01
> **Status**: done

## Goal

A one-shot subcommand that converts an existing `config.zsh` into the equivalent
`config.toml`, so the hand-maintained model tables are not retyped by hand.

## Files to create / modify

- `src/config.rs` (modify) — add the migration routine and its serialiser
- `src/main.rs` (modify) — wire `config migrate`

## Implementation notes

### Approach

Do not write a zsh parser. Run zsh and ask it what the values are:

1. Locate the source file — default
   `${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench/config.zsh`,
   overridable with a `--from <path>` flag.
2. Spawn `zsh -c` with a script that sources the file and then prints every setting in
   an unambiguous, parseable form. Scalars are easy; the two associative arrays and
   the order array need explicit iteration. Emit NUL- or tab-delimited records rather
   than relying on whitespace, since model labels contain spaces (`OpusPlan (Sonnet)`).
3. Parse that output into a **separate partial type**, then serialise to TOML.

Do **not** parse into the loader's `Config`. That struct has already applied defaults,
so by the time a value reaches it there is no way to tell "the user set this to the
default" from "the user never mentioned it" — and the requirement below is that absent
settings stay absent. Define a `PartialConfig` whose every field is an `Option`,
populate it only from what the dump actually reported, and serialise that. The loader's
`Config` is used only to *verify* the result, by loading the emitted TOML and comparing
resolved values.

The zsh dump must distinguish the two cases too: emit a record only for a variable that
is actually set, rather than emitting an empty value for one that is not.

**And the child shell must start clean.** "Is this variable set after sourcing?" only
means "did the file set it" if nothing else could have. Q's own environment may export
`Q_PROJECTS_ROOT` or either extra-args variable — several of them are designed to be
settable from the environment — and the migrator would then serialise a value the file
never mentioned.

So: `unset` every migrated setting at the top of the child script, before sourcing.
Keep `HOME`, `PATH` and `XDG_CONFIG_HOME`, which the file itself may reference. The
unset list is exactly the settings being migrated, so derive it from the same list the
dump iterates over rather than writing it twice.

Sourcing the file executes it. That is acceptable — it is the user's own file, and
sourcing is exactly what the zsh version did on every invocation — but say so in the
command's help text so it is not a surprise.

### Output behaviour

Write to stdout by default so the result can be reviewed before it is installed.
`--write` writes to the resolved `config.toml` path, refusing to overwrite an existing
file unless `--force` is given. Print the destination path on success.

### Fidelity

The two extra-args settings were space-split strings and become arrays: split on
whitespace, which reproduces exactly what zsh's `${=…}` did. Note in the emitted TOML
(as a comment) that an argument containing a space is now expressible and was not
before.

Preserve the model order list verbatim, including labels with spaces and parentheses.
Emit the two maps as TOML tables keyed by the label; quote every key, since
`OpusPlan (Sonnet)` is not a bare key.

If a setting is absent from the source file, omit it from the output rather than
writing the default — the loader supplies defaults, and an emitted default would
silently pin a value that should track future changes.

## Acceptance criteria

- [x] `workbench config migrate` reads the default `config.zsh` path and prints TOML
      to stdout.
- [x] `--from <path>` overrides the source; a missing source is a clear error.
- [x] `--write` writes to the resolved `config.toml` path and refuses to overwrite
      without `--force`.
- [x] Model labels containing spaces and parentheses survive as quoted TOML keys.
- [x] Extra-args strings become arrays split on whitespace.
- [x] Settings absent from the source are omitted, not defaulted — including a setting
      whose value happens to equal the built-in default.
- [x] Migration parses into an all-`Option` partial type, never into the loader's
      resolved config struct.
- [x] The child shell unsets every migrated setting before sourcing, so a value
      inherited from the environment is never mistaken for one the file set.
- [x] The command's help text states that the source file is executed by zsh.

## Verification

- [x] `cargo test` — a fixture `config.zsh` covering all scalars, both maps, the order
      array, a multi-flag extra-args string, one omitted setting, and one setting
      explicitly assigned its own default value, round-trips through migrate and then
      through the config loader to the expected values
- [x] `cargo test` — the emitted TOML contains a key for the explicitly-set default and
      no key for the omitted setting
- [x] `cargo test` — with a setting **exported in the migrator's own environment** but
      absent from the source file, the emitted TOML contains no key for it
- [x] Commit a **sanitised fixture** at `tests/fixtures/config.zsh` — modelled on Q's
      real 4.1 KB file but with no machine-specific paths — together with the expected
      resolved values, and make that the authoritative acceptance test. It must pass on
      any machine, with no file outside the repo.
- [x] Optional smoke test, when the file happens to exist: run against
      `~/.config/herdr/plugins/config/q.workbench/config.zsh` and compare the loaded
      values field by field against sourcing the original in zsh. Skip cleanly when it
      is absent — do not fail
- [x] `--write` twice: the second run refuses; with `--force` it succeeds
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Loses a setting, or mangles labels with spaces | Scalars fine, maps or omitted-setting handling wrong | Every setting round-trips; absent settings stay absent |
| Test coverage | ×2 | No fixture, or acceptance depends on a file outside the repo | Fixture covers scalars only | A committed fixture covers maps, spaces in labels, multi-flag args, omission and the inherited-variable case, and passes on any machine |
| Interface & readability | ×1 | Writes over the target with no guard | Guarded but no stdout-first default | Stdout by default, `--write`/`--force` guarded, destination printed |
| Assumptions & docs | ×1 | Silent about executing the source | Mentioned only in a code comment | Stated in the command help |

## Out of scope

- Migrating anything other than this plugin's settings.
- Keeping the command around forever — it is a one-shot tool, and may be dropped in a
  later release once no `config.zsh` remains anywhere.
