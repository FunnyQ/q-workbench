# POLISH-06: Remember the last harness and model per pane

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: agent/03, agent/04
> **Blocks**: cutover/01
> **Status**: todo

## Goal

Restarting an agent offers "use last combination" as the first harness-menu entry, so
the common case is one keypress, while still allowing a different harness or model.

## Files to create / modify

- `src/state.rs` (new) — the per-pane state file
- `src/flows/agent.rs` (modify) — record the choice; offer the extra entry
- `src/flows/restart.rs` (modify) — add `--restart` to the injected launcher command
- `src/main.rs` (modify) — the hidden `--restart` flag on `agent launch`

## Implementation notes

### Why state is needed at all

`pane.get` reports `agent` — which harness is running — but not the model. Restarting
"the same thing" therefore needs something remembered. Nothing else in this plugin
does, so keep the file as small as possible.

### Scope: per pane

Keyed by `pane_id`, so restarting a pane offers what *that* pane was running, not
whatever was launched most recently somewhere else.

Both stored values are the **pad-stripped menu label, glyph included** — byte for byte
what `AgentChoice.harness` and `AgentChoice.model_label` hold. The harness label
therefore begins with its Nerd Font codepoint; the model label does not carry one.
Using the raw label rather than a normalised identifier is what lets it resolve back
through the config maps when the menu is rebuilt.

```json
{"version":1,
 "panes":{"w2N:p1":{"harness":"󱖎  claude code",
                    "model":"Opus","recorded_at":1785474235}}}
```

That harness value is `U+F15CE` followed by two spaces and `claude code` — the exact
literal from the parity contract's glyph table. `model` is absent (or null) for codex
and opencode, which have no model menu.

Location: alongside the other state files, under
`$HOME/.local/state/herdr-workbench/last-agent.json`, overridable by an environment
variable for tests.

Written atomically, same as the registries: temp sibling, then rename.

A stored label that no longer exists must be handled: if the harness label is not one
of the three menu options, or the model label is absent from the config maps, drop the
entry and show the normal menus.

### Pruning

Pane ids are not durable across a Herdr restart, so the file would grow without bound.
On every write, drop entries whose `pane_id` is not in the current `pane.list`. That
costs one extra call on a path that is already doing several, and needs no separate
cleanup command.

### The menu entry

The menu lives in the launcher, not in the restart process — restart's only job is to
re-inject the launcher. Gate the extra entry on a **hidden `--restart` flag** that the
restart worker adds to the injected command.

Do **not** gate it on `--no-layout`. That is a public `agent launch` option, so a
manual no-layout launch would get the restart-only entry, and the sanctioned deviation
says the entry appears on restart. One hidden flag, one meaning.

The harness menu gains a first entry. Its literal is fixed in the parity contract's
glyph table: `U+F0709` — the same glyph the manifest's Restart Agent action uses —
followed by two spaces and `use last: <harness>`, and for claude ` · <model label>`.
`<harness>` here is the stored label **with its own glyph stripped**, so the entry does
not carry two glyphs.

Choosing it skips straight to the launch with no further menus. Choosing anything else
falls through to the existing flow unchanged.

When there is no usable entry for this pane, the menu is exactly as it is today. Do not
show a disabled or empty row.

The glyph is already recorded in the parity contract's glyph table, and `GLY-1` counts
19 literals rather than 18 because of it. Do not choose a different one.

### Recording — where, exactly

Not in the shared decision flow. The popup runs that flow **before** `tab.create`, so
at that point no agent pane exists and there is no `pane_id` to key on. Recording there
would also claim success before the tab was built or the harness launched.

Record in each entry point that actually holds a choice, after setup succeeds and the
pane id is known:

- **popup** — after `tab.focus`, keyed by the `root_pane.pane_id` returned by
  `tab.create`
- **in-pane launcher** — immediately before the `exec`, keyed by the pane id it was
  given. `exec` never returns on success, so this is the last possible moment

**Restart does not record.** It kills the agent and re-injects the launcher; the menus
then run *inside that injected process*, so the restart process never sees a choice and
would only be able to write stale values. The launcher covers the restart path already,
because the restart path is a launcher invocation.

To make that possible, `AgentChoice` must expose the two labels as fields, not only the
assembled argv:

```rust
pub struct AgentChoice {
    pub harness: String,          // the pad-stripped harness menu label
    pub model_label: Option<String>, // the pad-stripped model menu label; None for codex/opencode
    // …existing fields…
}
```

Both are the **menu labels**, matching what is stored.

## Acceptance criteria

- [ ] State is keyed by `pane_id` and stores the pad-stripped menu labels byte for
      byte, glyph included on the harness, with `model` absent for codex and opencode.
- [ ] The file is written atomically and pruned against the live pane list on write.
- [ ] The extra entry appears only when the launcher runs with the hidden `--restart`
      flag, and only when that pane has a usable record.
- [ ] Its literal matches the parity contract: `U+F0709`, two spaces, `use last: `, the
      stored harness label with its glyph stripped, and for claude ` · <model label>`.
- [ ] Choosing it launches with no further menus; choosing anything else behaves
      exactly as before.
- [ ] A stored model label that no longer resolves in the config drops the entry and
      falls through to the normal menus.
- [ ] Recording happens in the popup and the launcher only, after setup succeeds, keyed
      by the pane id that entry point knows — never inside the shared decision flow, and
      never in the restart process.
- [ ] `AgentChoice` exposes the harness label and the optional model label.
- [ ] A flow that fails before its pane is set up records nothing.
- [ ] A missing or corrupt state file is not an error — it means "no record".

## Verification

- [ ] `cargo test` — round-trip: record, read back, and confirm the stored harness
      value equals the glyph-bearing menu label byte for byte and the model label
      resolves through the config maps
- [ ] `cargo test` — a codex launch records no `model`, and its extra entry offers the
      harness alone
- [ ] `cargo test` — pruning removes entries absent from the pane list and keeps the rest
- [ ] `cargo test` — a stored label missing from the config yields no extra entry
- [ ] `cargo test` — the launcher without `--restart` never shows the extra entry, even
      when a record exists for its pane and even with `--no-layout` set
- [ ] `cargo test` — the restart flow writes nothing to the state file
- [ ] `cargo test` — a corrupt state file yields no record and no error
- [ ] `cargo test` — with `FakeClient`, choosing the extra entry issues no `gum` model
      prompt and reaches the launch directly
- [ ] `cargo test` — a popup flow that fails after `tab.create` records nothing
- [ ] `cargo test` — a successful popup flow records against the `root_pane.pane_id`
      returned by `tab.create`, not any other id
- [ ] Manual in a **scratch tab**: launch an agent, restart it, take the first entry,
      and confirm the same harness and model come back with one keypress
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Stores the resolved model instead of the label, or the extra entry appears outside restart | Works but the file grows unbounded, or a stale label breaks the menu | Per-pane, label-based, pruned, graceful on stale or corrupt state |
| Test coverage | ×2 | No state tests | Round-trip only | Round-trip, pruning, stale label, corrupt file, and the skip-the-menus path |
| Interface & readability | ×1 | State reading scattered through the flows | One module but the menu logic duplicated | One small state module; the menu gains one conditional entry |
| Assumptions & docs | ×1 | No note on pane-id durability | Mentioned briefly | Explains why pruning is needed and why the label rather than the value is stored |

## Out of scope

- Remembering the worktree or the usage label. Restart already reuses the pane's label
  and never creates a worktree.
- A global "last launched" record. This is deliberately per pane.
