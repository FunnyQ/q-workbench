# PICKER-01: `workbench project source`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: registry/02
> **Blocks**: picker/02, polish/04
> **Status**: done

## Goal

Emit the registered projects as NUL-delimited fzf records, plus the best zoxide match
for an active query, fast enough to run on every keystroke.

## Files to create / modify

- `src/flows/picker.rs` (new) — the source emitter
- `src/main.rs` (modify) — wire `project source [query]`

## Implementation notes

### Why speed matters here

The project picker binds `change:reload(<self> source {q})`, so this subcommand runs
**once per keystroke**. It is the single strongest reason the plan moved off the
`herdr` CLI and `jq`. It must do no Herdr call at all — it reads the registry file and
nothing else.

### Record format

Each record is **three** display lines; the payload is appended to the end of the third
one, not put on a line of its own:

```
<U+F024B>  <name>[ | <aliases joined by " | ">]
   <path with $HOME collapsed to ~>
   <sources joined by " · "><TAB><absolute path><NUL>
```

A fourth line would change both the bytes and how fzf renders the record — the source
script appends `"\t\(.key)\u0000"` directly to the sources line.

The leading three spaces on lines two and three are literal. The separator between
sources is a space, a middle dot (U+00B7), and a space. The alias separator is a
space, a pipe, and a space.

Hidden entries are excluded. Sort order is the same as the SSH list: entries with a
`last_used_at` first, most recent first; then the rest ordered by **display name**
(note: by name here, unlike the SSH registry which orders by key).

### zoxide fallback

Only when the query is at least two characters, and only after the registry rows:

1. `zoxide query -- <query>`; a non-zero exit or empty output means emit nothing
2. the result must be an existing directory; resolve symlinks
3. if that resolved path is already a registered project, emit nothing
4. otherwise emit one record with `zoxide` as its only source line and the directory's
   basename as its name

`zoxide` missing from `PATH` is not an error — emit nothing extra.

### Registry absent

Exit non-zero with no output, as the zsh version does. The picker itself reports the
missing registry.

## Acceptance criteria

- [x] Records match the format above exactly: three display lines, the payload appended
      to the third, the leading three spaces, the middle-dot source separator, and the
      NUL terminator.
- [x] Hidden entries are excluded.
- [x] Sort is most-recently-used first, then never-used by display name.
- [x] `$HOME` is collapsed to `~` in the displayed path but the payload stays absolute.
- [x] The zoxide row appears only for a query of two or more characters, only for an
      existing directory, and only when that directory is not already registered.
- [x] A missing `zoxide` binary is not an error.
- [x] A missing registry exits non-zero with no output.

## Verification

- [x] `cargo test` — golden-output test against a fixture registry with a hidden
      entry, an aliased entry, a used and an unused entry; compare bytes including NULs
- [x] `cargo test` — the `~` collapse applies to the display line only
- [x] `cargo test` — zoxide fallback suppressed for a one-character query, for a
      nonexistent path, and for an already-registered path
- [x] **Byte-identical check**: run the zsh `scripts/project-picker-source.zsh` and the
      Rust `project source` against the same fixture registry and compare with `cmp`
- [x] Measure per-invocation cost against the real registry: run 50 warm invocations,
      take the **median** wall time, and require **≤ 5 ms**. The zsh version measured
      14.6 ms; anything at or above that means the rewrite lost its main motivation on
      this path and should be raised rather than absorbed
- [x] `cargo clippy -- -D warnings` is clean

### Measured cost

Run `zsh scripts/bench-project-source.zsh`. It builds the release binary and takes the
median of 50 warm invocations against the real registry, timing each one with zsh's
`EPOCHREALTIME` builtin. **Measure the release binary, and use a harness that spawns
nothing per sample**: a `python`, `date` or `time` wrapper adds 2-3 ms of its own
fork+exec to every sample, which is most of the budget.

Measured on the target machine, 60-entry registry (2026-07-31):

| Command | Median |
|---|---|
| `/usr/bin/true` — the harness's own fork+exec floor | 2.8 ms |
| `workbench project source` | **3.9 ms** |
| `zsh scripts/project-picker-source.zsh` | 15.5 ms |
| `workbench project source <query>` | 12.0 ms |
| `zsh scripts/project-picker-source.zsh <query>` | 31.8 ms |

So the subcommand costs about 1.2 ms beyond the fork+exec every process pays, and a
bare Rust binary that does nothing at all costs 0.7 ms of that. With a query of two or
more characters both versions shell out to `zoxide query`, which is 8 ms on its own and
dominates the number; that call is required for parity.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Record bytes differ, or hidden entries leak in | Format right but sort order or the zoxide guards differ | Byte-identical output including NULs; every guard reproduced |
| Test coverage | ×2 | No golden test | Golden test only | Golden bytes, sort order, all three zoxide suppression cases, and the byte-identical comparison |
| Interface & readability | ×1 | Formatting inlined in the command handler | Extracted but takes I/O | A pure function from registry data to bytes, tested directly |
| Assumptions & docs | ×1 | No note on per-keystroke cost | Mentioned without a measurement | Cost measured and recorded, with the keystroke path explained |

## Out of scope

- The fzf invocation and the key bindings — the picker task owns those.
- Any Herdr call. This subcommand must not make one.
