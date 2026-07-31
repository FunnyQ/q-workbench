# FOUNDATION-01: Cargo skeleton, clap dispatch, and the build script

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
>
> **Depends on**: none — foundation task
> **Blocks**: foundation/02, foundation/03, foundation/06, registry/01, registry/04, agent/01
> **Status**: todo

## Goal

A buildable Rust crate whose `workbench` binary parses every planned subcommand and
exits cleanly, plus the release script that produces the committed `bin/workbench`.

## Files to create / modify

- `Cargo.toml` (new) — crate manifest, `[[bin]] name = "workbench"`
- `src/main.rs` (new) — clap dispatch, every subcommand stubbed
- `scripts/build.zsh` (new) — `cargo build --release` then copy to `bin/workbench`
- `.gitignore` (modify) — add `target/`
- `rustfmt.toml` (new, optional) — only if a non-default is actually needed

## Implementation notes

### Crate

Edition 2021. Dependencies, all with `default-features` left alone unless a build
problem appears:

```toml
clap       = { version = "4", features = ["derive"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
toml       = "0.8"
anyhow     = "1"
libc       = "0.2"
time       = { version = "0.3", features = ["formatting"] }
```

Set `[profile.release] strip = true` and `opt-level = "z"` only if the measured binary
size in the acceptance criteria comes out above 5 MB. Measure before tuning — the
premise is 2–3 MB untuned and it is worth knowing the real number.

### Subcommand tree

Model it with clap derive. The full surface is listed in the shared context.

Every leaf must parse and, for now, return
`Err(anyhow!("unimplemented: {}", subcommand_path))`. `main` prints that message to
stderr and exits 1. Do not call `process::exit` inside a leaf — one dispatch shape,
one exit path, and later tasks replace the `Err` with real work. That makes an
accidentally shipped stub loud rather than silent.

The top-level groups are `agent`, `project`, `ssh`, `dashboard`, `config`, `herdr`.
`agent launch` and `agent inject` take a positional `<pane_id>` plus `--tab`,
`--usage`, `--worktree`, and (`launch` only) `--no-layout`. `config migrate` takes
`--from <path>`, `--write` and `--force`. `agent restart-worker` takes `--pane
<pane_id>` and is marked `#[command(hide = true)]` — it is spawned by `agent restart`,
never typed. `agent launch --restart` is likewise hidden. `ssh edit` takes an
**optional** positional target, because the picker binding passes an empty string when
nothing is selected. Do **not** reproduce the old positional-slot
convention where empty quoted arguments held their place.

Define every flag now, even though the behaviour lands later — a later task should not
have to reopen the CLI definition to add one.

Add `#[command(name = "workbench")]` so help output and `current_exe()`-derived
strings agree.

### Build script

```zsh
#!/usr/bin/env zsh
set -eu
cd "${0:A:h:h}"
cargo build --release
mkdir -p bin
cp target/release/workbench bin/workbench
print -r -- "built bin/workbench ($(du -h bin/workbench | cut -f1))"
```

`bin/workbench` is committed; `target/` is not.

### Cold-start benchmark

The plan rests on Rust starting in roughly 1–5 ms. A hello-world measured 3.6 ms, but
the real binary links `clap` and `serde`. Measure the actual figure now, while the
binary is still trivial, and again is not needed later — record it in the task's
completion note. A simple loop is enough:

```zsh
time (repeat 50 ./bin/workbench --version >/dev/null)
```

Divide by 50. If the per-invocation cost exceeds ~15 ms — the measured `bun run`
figure — that invalidates a premise of the plan and must be raised rather than worked
around.

## Acceptance criteria

- [ ] `cargo build --release` succeeds with no warnings.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `workbench --help` lists the `agent`, `project`, `ssh`, `dashboard`, `config`
      and `herdr` groups.
- [ ] Every leaf subcommand in the shared context's surface parses, including every
      flag listed there; an unknown one fails with a clap error, not a panic.
- [ ] `zsh scripts/build.zsh` produces `bin/workbench` and prints its size.
- [ ] `target/` is gitignored; `bin/workbench` is not.
- [ ] Measured per-invocation cost and binary size are recorded.

## Verification

- [ ] `cargo build --release && cargo clippy -- -D warnings`
- [ ] `zsh scripts/build.zsh && ./bin/workbench --help`
- [ ] `cargo test` — a **table-driven parse test** with one row per leaf in the shared
      context's surface, each row an argv vector exercising that leaf with all of its
      flags set. Every row must parse. Add rows for an unknown subcommand, an unknown
      flag on a known leaf, and a missing required positional; each must produce a clap
      error rather than a panic. `--help` alone does not verify nested leaves, so this
      table is the criterion's only real check.
- [ ] `./bin/workbench agent launch w1:p1 --usage test` prints
      `unimplemented: agent launch` to stderr and exits 1
- [ ] `./bin/workbench ssh edit` (no target) parses rather than failing with a clap
      missing-argument error
- [ ] `time (repeat 50 ./bin/workbench --version >/dev/null)` — record the per-call figure
- [ ] `git status --short` shows `bin/workbench` as an added file and no `target/` entries

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Does not build, or the subcommand surface diverges from the planned one | Builds, but some leaves are missing or the build script does not produce `bin/workbench` | Full surface parses, build script works, artifact committed and `target/` ignored |
| Test coverage | ×2 | No verification run | Build checked, but leaves verified only through `--help` | Build, clippy, a table-driven test covering every leaf and its flags plus the three error rows, and the startup measurement recorded |
| Interface & readability | ×1 | Dispatch logic tangled into `main` | Clap types defined but argument naming inconsistent | Clean derive types, `main.rs` dispatches only |
| Assumptions & docs | ×1 | Binary size and startup unmeasured | Measured but not written down | Both recorded, and a startup figure above ~15 ms raised as a blocker rather than absorbed |

## Out of scope

- Any real behaviour behind the subcommands — every later task fills one in.
- Release-profile size tuning unless the measured binary exceeds 5 MB.
