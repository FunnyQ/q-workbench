# REGISTRY-01: Project discovery and canonicalisation

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/03
> **Blocks**: registry/02
> **Status**: todo

## Goal

A pure discovery layer that finds candidate project paths from the three sources and
canonicalises them, with no registry file involved.

## Files to create / modify

- `src/registry/project.rs` (new) — `canonical_project()` and the three discovery
  functions

## Implementation notes

This task produces no CLI subcommand and touches no registry file. It exists on its own
because the discovery rules are the part most likely to drift silently, and they are
testable in isolation against a fixture tree.

### `canonical_project()`

The rules are inlined in the parity contract: resolve to the git toplevel, resolve
symlinks, reject `/`, and drop anything under `/tmp`, `/private/tmp`,
`/var/folders/*/*/T` or `/private/var/folders/*/*/T` **unless** it sits inside the
resolved projects root.

That last exception matters and is easy to lose: a developer whose projects root is
itself a symlink into a temp-like path would otherwise have every project silently
dropped.

A path that fails any rule yields nothing. This is not an error — a stale session
pointing at a deleted directory is the normal case, not a fault.

```rust
fn canonical_project(path: &Path, projects_root: &Path) -> Option<PathBuf>
```

Take the projects root as a parameter rather than reading config inside, so the
function stays pure and the exception is directly testable.

### The three sources

Each returns `Vec<(PathBuf, Source)>`; the caller merges. Sources are `claude`,
`codex`, `filesystem`.

- **Claude** — walk `~/.claude/projects` for `sessions-index.json` and read
  `.entries[].projectPath`; also scan the `*.jsonl` transcripts for `cwd`. The zsh
  version used `rg -m1` with a regex over escaped JSON strings because fully parsing
  large transcripts was too slow. Match that: read line by line and stop at the first
  `cwd` found per file.
- **Codex** — the first line of every `~/.codex/sessions/**/rollout-*.jsonl`; take
  `payload.cwd` when `type == "session_meta"`, else `cwd`.
- **Filesystem** — walk the projects root for `.git`, pruning `node_modules`, `vendor`,
  `tmp`, `log`, `coverage`, `dist`, `build`, `.nuxt`, `.next`. Prune **before**
  descending; do not walk into a pruned directory and filter afterwards, or a large
  `node_modules` will dominate the runtime.

A missing source directory yields an empty list, not an error.

## Acceptance criteria

- [ ] `canonical_project` resolves to the git toplevel, resolves symlinks, rejects `/`,
      drops temp-directory paths, and keeps a temp-like path that is inside the
      projects root.
- [ ] It takes the projects root as a parameter and reads no configuration itself.
- [ ] Each of the three sources returns its paths tagged with the right source name.
- [ ] Claude discovery reads both `sessions-index.json` entries and transcript `cwd`
      values, stopping at the first `cwd` per transcript.
- [ ] Codex discovery reads only the first line of each rollout and handles both the
      `session_meta` and the plain `cwd` shapes.
- [ ] Filesystem discovery prunes before descending.
- [ ] A missing source directory yields an empty list.

## Verification

- [ ] `cargo test` — `canonical_project` over a fixture tree: a git subdirectory
      resolving to its toplevel, `/`, a `/tmp` path, a `/var/folders/.../T` path, a
      symlinked path, and a temp-like path inside the projects root (which must be kept)
- [ ] `cargo test` — Claude discovery against a fixture with one `sessions-index.json`
      and two transcripts, one of which has several `cwd` occurrences
- [ ] `cargo test` — Codex discovery against a fixture with one `session_meta` rollout
      and one plain-`cwd` rollout
- [ ] `cargo test` — filesystem discovery against a fixture containing a `.git` inside
      a `node_modules`, asserting it is not returned
- [ ] `cargo test` — all three return empty for a missing directory
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | The temp-dir filter or its projects-root exception is missing | Filter right but a discovery source misses a shape | Every filter rule and every source shape reproduced |
| Test coverage | ×2 | No fixture tree | Filter tested, sources not | All six filter cases and all three sources covered, including the pruning case |
| Interface & readability | ×1 | Reads config internally, hard to test | Pure but the three sources share tangled helpers | Pure functions with the root passed in; each source independently testable |
| Assumptions & docs | ×1 | Filter rules uncommented | Listed without reasons | The temp-dir filter, its exception, and the line-by-line transcript scan all explained |

## Out of scope

- The registry file, merging, and any CLI subcommand — later tasks in this bucket.
