# CUTOVER-02: Documentation and release wiring

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: cutover/01
> **Blocks**: cutover/03
> **Status**: todo

## Goal

Bring every document in the repo in line with the Rust implementation, and wire the
version files so the crate and the manifest cannot drift.

## Files to create / modify

- `README.md` (modify) — the claims, the dependency list, the commands, the config section
- `CLAUDE.md` (modify) — rewritten architecture notes
- `.chronicle/release.json` (modify) — add `Cargo.toml` to `versionFiles`
- `.gitignore` (modify) — confirm `target/` present, remove anything now stale

## Implementation notes

### README

Concrete corrections, all of which are now false or out of date:

- `README.md:5` claims "Pure zsh. No build step, no dependencies to install beyond the
  CLI tools below." Replace with an accurate statement: a Rust binary is committed, so
  installing still needs no build, but **hacking on it does**.
- The requirements section lists `jq` among the tools on `PATH`. `jq` is no longer
  used. `gum`, `fzf`, `zoxide`, `yazi` and `trash` remain. `rg` is no longer used
  either — confirm before removing it.
- The registry sections invoke `scripts/project-registry.zsh scan` and
  `scripts/ssh-target-registry.zsh sync`. These become `./bin/workbench project scan`
  and `./bin/workbench ssh sync`.
- The configuration section describes copying `config.example.zsh` and the zsh
  sourcing model, including the `typeset -gA` warning. Replace with the TOML file, the
  `workbench config migrate` path for existing users, and the settings table updated
  for the array-valued extra args.
- The development section describes the zsh test suite. Replace with `cargo test`,
  `cargo clippy -- -D warnings`, and `zsh scripts/build.zsh`.
- The `herdr plugin link` note at `README.md:36` says "edits to `scripts/` take effect
  on the next invocation, no reinstall". That is no longer true — say plainly that an
  edit needs `zsh scripts/build.zsh` before it takes effect. This is the single most
  likely source of confusing stale-binary bugs, so it belongs in the install section,
  not a footnote.

Keep the bypass-flag warning. It is still accurate and still important.

### CLAUDE.md

This file is the architecture note for future sessions and describes a codebase that
no longer exists. Rewrite it around the Rust structure, carrying forward the reasoning
that is still true — the deferred split, the `exec` dependency, the popup cwd trap, the
NUL-delimited picker records, the temp-dir filter — and dropping what is not: the
`typeset -gA` trap, the jq parsing conventions, the "pure zsh, no build step" framing,
and the two-agent-launch-paths warning that consolidation removed.

Add what is new and non-obvious: the one-request-per-connection socket contract, the
multi-chunk buffering requirement, the `pane.send_input` key vocabulary, the fact that
`bin/workbench` is a committed artifact needing a rebuild, and the shell-quoting
boundary in the restart injection.

Match the existing file's comment density and tone: it explains *why*, not *what*.

### Release wiring

`.chronicle/release.json` currently lists only `herdr-plugin.toml` as a version file.
Add `Cargo.toml` with the pattern matching its `version = "…"` line, so a release bumps
both. Verify the pattern actually matches the crate manifest's formatting — the two
files use the same `version = "x.y.z"` shape, but confirm rather than assume.

**The binary must be rebuilt and committed as part of any release**, since the version
it reports comes from the crate. Note that requirement in the release section of the
README so it is not discovered after a tag is pushed.

## Acceptance criteria

- [ ] Every false claim listed above is corrected in `README.md`.
- [ ] The dependency list matches what the binary actually invokes.
- [ ] Every documented command uses the new subcommand form.
- [ ] The configuration section documents TOML, the migration path, and array-valued
      extra args, and keeps the bypass-flag warning.
- [ ] The rebuild requirement is stated in the install section, not buried.
- [ ] `CLAUDE.md` describes the Rust architecture, carries forward the still-true
      reasoning, and drops what no longer applies.
- [ ] `.chronicle/release.json` includes `Cargo.toml` with a pattern that matches.
- [ ] The release section states that the binary must be rebuilt and committed.

## Verification

- [ ] Read `README.md` end to end and check every command against the built binary by
      running it
- [ ] Confirm no document references a deleted `scripts/*.zsh` path:
      `rg -n 'scripts/[a-z-]+\.zsh' README.md CLAUDE.md` returns only `build.zsh`
- [ ] Confirm no document references `jq` or `config.example.zsh`
- [ ] Dry-run the release version bump and confirm both `herdr-plugin.toml` and
      `Cargo.toml` are updated
- [ ] Have a fresh reader follow the install and configure sections on a clean setup

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A documented command does not work, or a false claim survives | Commands right but the dependency list or the rebuild note is wrong | Every command verified by running it; every claim true |
| Test coverage | ×2 | Nothing verified | Commands skimmed | Every command run, both greps clean, version bump dry-run checked |
| Interface & readability | ×1 | `CLAUDE.md` is a diff of the old one | Rewritten but loses the reasoning worth keeping | Reads as written for the Rust codebase, with the still-true traps carried across |
| Assumptions & docs | ×1 | Rebuild requirement omitted | Mentioned in passing | Stated in the install section and in the release section |

## Out of scope

- Cutting the release itself. That is a separate, human-initiated step.
- Writing new user-facing features into the docs.
