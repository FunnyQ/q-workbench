# REGISTRY-03: Interactive project review and edit

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: registry/02
> **Blocks**: polish/04
> **Status**: todo

## Goal

`workbench project scan`, `rescan` and `edit` — the three operations that prompt.

## Files to create / modify

- `src/registry/project.rs` (modify) — the three interactive operations
- `src/main.rs` (modify) — wire `project scan`, `project rescan`, `project edit`

## Implementation notes

### `scan` and `rescan`

Both build a candidate list from discovery plus the existing registry, present it, and
write only what was selected.

Row format, from the parity contract's marker rules:

- `scan` — `<name>\t<path>`; refuses to run when the registry already exists
- `rescan` — `[new] <name>\t<path>` for a discovered path not in the registry,
  `[missing] <name>\t<path>` for a registered path no longer discovered, and plain
  `<name>\t<path>` otherwise; requires an existing registry

Present with:

```
gum choose --no-limit --selected='*' --ordered --height=24 --no-strip-ansi \
  --header='Review projects (space: toggle · enter: save)'
```

Keep every flag. `--no-strip-ansi` matters because the markers are meant to survive,
and `--selected='*'` is what makes "enter accepts everything" the default gesture.

From the selection, keep only the **last** tab-separated field — that is the path. The
display column may itself contain no tab, but parsing from the end is what the zsh
version does and is robust to a name containing one.

Cancelling, or selecting nothing, leaves the registry unwritten and exits 1 with the
message from the parity contract. This is the case most worth getting right: a `scan`
that writes an empty registry destroys the user's project list.

### `edit`

Three prompts in order — display name, comma-separated aliases, and a visible/hidden
choice seeded with the current value:

```
gum input  --header='Display name'                --value=<current>
gum input  --header='Aliases (comma-separated)'   --value=<current, joined by ", ">
gum choose visible hidden --header='Picker visibility' --selected=<current>
```

Normalise aliases by trimming each, dropping empties, and deduplicating **while
preserving order**. An empty display name falls back to the path's basename.

Cancelling any prompt leaves the registry unwritten and exits 1.

`edit` is also invoked from the project picker's `ctrl-i` binding, so it must `clear`
before drawing — fzf owns the alternate screen and needs it clean to redraw when the
command exits.

## Acceptance criteria

- [ ] `scan` refuses an existing registry; `rescan` and `edit` require one.
- [ ] `rescan` marks `[new]` and `[missing]` exactly as specified.
- [ ] The `gum choose` invocation keeps every flag and the exact header text.
- [ ] Only the last tab-separated field of each selected row is used as the path.
- [ ] Cancelling, or selecting nothing, leaves the registry unwritten and exits 1 with
      the parity contract's message.
- [ ] `edit` prompts in the documented order with the documented seeds.
- [ ] Aliases are trimmed, de-emptied and deduplicated with order preserved; an empty
      name falls back to the basename.
- [ ] The screen is cleared before drawing, so the fzf binding redraws correctly.

## Verification

- [ ] `cargo test` — candidate-row construction for `scan` and for all three `rescan`
      cases, asserting the exact marker text
- [ ] `cargo test` — selection parsing, including a display name containing a tab
- [ ] `cargo test` — alias normalisation: whitespace, empties, duplicates, order
- [ ] `cargo test` — cancellation and empty selection both leave a fixture registry
      byte-for-byte unchanged
- [ ] Manual: run `project rescan` against a copy of the real registry and confirm the
      markers match what the zsh version produces for the same inputs
- [ ] Manual: invoke `edit` through the picker's `ctrl-i` binding and confirm fzf
      redraws cleanly afterwards
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Cancelling writes the registry, or a marker differs | Markers right but alias normalisation or a guard drifts | Every marker, flag, prompt and guard reproduced |
| Test coverage | ×2 | No row tests | Rows only | Rows, selection parsing, alias normalisation, and both leave-unwritten cases |
| Interface & readability | ×1 | Row building mixed with prompting | Separated but the parse is fragile | Row construction pure and tested; prompting thin |
| Assumptions & docs | ×1 | No note on the `clear` requirement | Mentioned in passing | Explains fzf's screen ownership and why an empty selection must never be written |

## Out of scope

- Discovery and the store — both already exist and are used here.
- The picker itself, which merely invokes `edit` from a binding.
