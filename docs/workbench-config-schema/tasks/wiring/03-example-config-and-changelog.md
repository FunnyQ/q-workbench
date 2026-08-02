# WIRING-03: Example config, changelog, and prose docs

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: launch/02, wiring/01
> **Blocks**: wiring/04
> **Status**: done

## Goal

Every document in the repository describes the two-section schema that now exists, and the breaking change is recorded in the changelog.

## Files to create / modify

- `config.example.toml` (modify) — final pass: confirm the removed section is gone, document how a layout is selected.
- `CHANGELOG.md` (modify) — an `## [Unreleased]` section with the breaking change and the removals.
- `CLAUDE.md` (modify) — the configuration paragraphs are now false and must be replaced.
- `README.md` (modify) — the whole Configuration section and the actions table.

## Implementation notes

**No Rust in this task.** If you find yourself opening a file under `src/`, it is only to confirm a claim you are about to write down.

### Nerd Font glyphs — read this before editing anything

`config.example.toml`, `herdr-plugin.toml`, and `README.md` all carry plane-15 Nerd Font glyphs. Three rules:

- Do **not** retype or paste a glyph. Use the repo's `unicode-edit` skill for any line carrying one.
- Do **not** write those files with a bash heredoc. A heredoc drops the fifth hex digit, so U+F169F becomes U+F169 followed by a literal `f`. TOML and Markdown both parse the result without complaint, and the corruption only surfaces when you dump codepoints per character.
- Prefer edits that do not touch a glyph-bearing line at all. Most of this task is prose, so this is usually achievable.

After editing, verify with:

```zsh
python3 -c "
for p in ['config.example.toml','herdr-plugin.toml','README.md','CLAUDE.md','CHANGELOG.md']:
    for c in open(p).read():
        if ord(c) > 0xFFFF: print(p, hex(ord(c)))
" | sort -u
```

Every plane-15 value printed must be an intended glyph. A bare `0xf169` is corruption.

### `config.example.toml`

This file is the executable specification for the whole schema, already written in a prior session. This task is a final pass, not a rewrite:

1. Confirm the `[[workspaces]]` section is gone. It was cut from scope, so no entry, no header, and no comment referring to it may remain.
2. Confirm `dashboard_workspace`'s comment says it names a **Herdr workspace label directly** — the workspace whose label Herdr reports in `workspace.list`. It does not name a config entry. The old comment said "Names a `[[workspaces]]` entry", which is now wrong.
3. Under the tab-layouts header, add a short note on how a layout is reached:

   > A layout is selected with `--layout <name>` — `workbench agent popup --layout personal-assistant`, or a Herdr action whose command carries the flag. Without the flag the launcher uses `default_tab_layout`.

4. Leave every other comment, value, and glyph exactly as it is. The argv lines and pane geometry in this file are the parity baseline the test suite compares against.

### `CHANGELOG.md`

Add an `## [Unreleased]` section directly above `## [0.3.0] - 2026-08-02`. Do not bump any version and do not touch the existing entries — the release skill owns versioning.

Follow the file's established voice: complete sentences that explain the user-visible effect, not bare identifiers. Two subsections:

**`### Changed`** — lead with **BREAKING**. Describe the replacement: the flat model and extra-argument settings are gone, replaced by two declarative sections that describe tab layouts and agents. State that a layout which omits a choice asks for it, so a layout pinning nothing behaves exactly as before. State plainly that there is no automatic conversion from the old file, and that an old config now fails to load with an unknown-field error naming the offending key.

**`### Removed`** — two items:

- The zsh-to-TOML conversion subcommand, and why: the zsh era is over and the command's only user has already converted.
- The five environment-variable overrides that belonged to the deleted settings: `Q_AGENT_MODEL_ORDER`, `Q_AGENT_MODELS`, `Q_AGENT_MODEL_ARGS`, `Q_CLAUDE_EXTRA_ARGS`, `Q_CODEX_EXTRA_ARGS`. Note that the path settings and `Q_DASHBOARD_WORKSPACE` keep their environment overrides.

Naming the removed identifiers here is correct and required — a reader hitting the breakage needs to recognise their own config.

### `CLAUDE.md`

Its "CLI and configuration" section contains two paragraphs that are now false.

The first says bypass flags are kept opt-in "through `claude_extra_args` and `codex_extra_args`". Those fields no longer exist. Replace with: bypass flags stay opt-in through an agent's `extra_args` array in the config; nothing adds them unconditionally.

The second describes the zsh migration subcommand as "the only compatibility boundary with the old zsh config", including the note about executing the source file. That command is gone — delete the paragraph outright rather than rewriting it.

In their place, document what a maintainer now needs to know:

- The config carries two array-of-tables sections. One describes tab layouts and their panes; one describes agents and their options.
- Omitting a pane's agent, a pane's option, or a layout's tab label means the launcher asks for that choice at launch time.
- **All validation runs at config load, before the first socket call.** State the reason: the popup path closes its tab when construction fails, but the in-pane path has no such cleanup, so a half-built layout would be left on screen.
- A layout is selected with `--layout <name>`, defaulting to `default_tab_layout`.

Also scan the rest of `CLAUDE.md` for stale claims. Its "Commands" list and its "Agent launch and restart" section were written against the hardcoded layout; correct anything that now reads as false, but do not rewrite sections that are still accurate.

### `README.md`

More work than it looks. Two areas:

**The actions table** (currently seven rows) gains a row for the new pinned-layout action:

| Action | What happens |
| --- | --- |
| `new-assistant` | Open a tab from the `personal-assistant` layout — every choice pinned, so no menus |

The sentence below the table reads "Harnesses offered: Claude Code (Opus / OpusPlan / CCR / Fable 5), Codex, opencode." Reword it so those are the *defaults*, and say they are configurable. The keybinding table further down is Q's personal set — add a row for the new action only if it reads naturally; leaving it out is fine.

**The Configuration section** is almost entirely stale and needs rewriting:

- The migration snippet showing the two conversion invocations must go.
- The `Q_WORKBENCH_LOCAL_CONFIG` paragraph describes recovering from the zsh cutover. That override still exists and still redirects the config path, so keep the fact, but drop the migration framing.
- The example TOML block sets the two deleted extra-argument arrays. Replace it with a short block showing the shape that now exists: `dashboard_workspace`, `default_tab_layout`, and a minimal `[[agents]]` entry with one option.
- The settings table has rows for `claude_extra_args`, `codex_extra_args`, and the combined model-menu row. Delete those three rows. Keep `dashboard_workspace` and the four path settings. Add a `default_tab_layout` row.
- The bypass-flags paragraph points at the two deleted arrays. Rewrite it to point at an agent's `extra_args`, keeping its warning intact: those flags hand the agent unrestricted execution on the host, and nothing adds them for you.
- Point readers at `config.example.toml` as the full reference rather than duplicating the whole schema in the README.

## Acceptance criteria

- [x] `config.example.toml` has no `[[workspaces]]` section and no comment referring to one; `dashboard_workspace` is documented as naming a Herdr workspace label directly.
- [x] `config.example.toml` documents `--layout <name>` and the `default_tab_layout` fallback.
- [x] `CHANGELOG.md` has an `## [Unreleased]` section above the 0.3.0 entry, with a **BREAKING** `### Changed` item and a `### Removed` item naming the subcommand and all five environment variables.
- [x] No version number changed anywhere.
- [x] `CLAUDE.md` no longer describes the zsh migration subcommand or the two deleted extra-argument fields, and states that all validation runs at config load before the first socket call.
- [x] `README.md`'s actions table includes the new action, and its Configuration section documents only settings that exist.
- [x] Outside `docs/` and the changelog's Removed entry, no Markdown or TOML file in the repository mentions the removed subcommand, the two deleted extra-argument fields, the deleted model-argument field, or a `Q_AGENT_MODEL` variable.
- [x] No plane-15 codepoint in any edited file was altered.

## Verification

- [x] Run the codepoint dump shown above and confirm no `0xf169` appears.
- [x] Run:
      `rg -n 'config migrate|claude_extra_args|codex_extra_args|model_args|Q_AGENT_MODEL' --glob '!docs/**' --glob '*.md' --glob '*.toml' .`
      Expect hits only inside the changelog's Removed entry. Any other hit is a stale document.
- [x] `git diff --stat CHANGELOG.md` shows additions only above the 0.3.0 heading, and no change to any existing entry.
- [x] Read `config.example.toml` end to end and confirm the argv-bearing `[[agents]]` entries and the pane geometry values are untouched — the test suite compares against them.
- [x] `cargo test` passes. It should be unaffected, which is the point: if a documentation change broke a test, a test was asserting on prose.
- [x] Run `git status --short` and quote it. Expect `config.example.toml`, `CHANGELOG.md`, `CLAUDE.md`, `README.md`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A document still instructs the reader to use a removed setting or subcommand, or a glyph was corrupted | Most staleness fixed but the README settings table or the maintainer notes still describe deleted fields | Every document describes only what exists; the breaking change is unambiguous; glyphs byte-identical |
| Test coverage | ×2 | No check that the staleness is actually gone | The grep was run but scoped too narrowly to catch every file | The full grep and the codepoint dump both run clean, and the specification file is confirmed unchanged where it matters |
| Interface & readability | ×1 | Changelog entry is a list of bare identifiers | Prose exists but buries the breakage | Leads with BREAKING, tells the reader what will fail and what to write instead, in the file's established voice |
| Assumptions & docs | ×1 | The removals are unexplained | Removals listed without a reason | Each removal says why, and points at the replacement |

## Out of scope

- Bumping the version or writing a release heading — Deferred. The release skill owns versioning; this task only writes under `## [Unreleased]`.
- Documenting the deferred workspace feature — Deferred. It has no implementation to describe.
- Rewriting the README's Registries or Install sections — Deferred. They are unaffected by the schema change.
