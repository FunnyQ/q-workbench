# LAUNCH-03: Side panes from the layout

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: config/04
> **Blocks**: wiring/01
> **Status**: todo

## Goal

`build_side_panes()` iterates the layout's pane list instead of making five literal socket calls, so any pane arrangement a user writes in TOML is built — while the default layout still produces today's exact call sequence.

## Files to create / modify

- `src/flows/agent.rs` (modify) — `build_side_panes`, its two call sites (`apply_launch_layout`, `build_popup_tab`), `create_popup_tab`'s hardcoded root env, and the pane-building tests.

## Implementation notes

### What already exists

The config loader is finished and validated. These types are available and every reference in a loaded `Config` is guaranteed to resolve — an executor **must not** re-validate them:

```rust
pub struct TabLayout { pub name: String, pub tab_label: Option<String>, pub panes: Vec<LayoutPane> }
pub struct LayoutPane {
    pub name: String, pub label: Option<String>, pub icon: Option<String>,
    pub pane_type: PaneType, pub agent: Option<String>, pub option_name: Option<String>,   // TOML key: option
    pub command: Option<String>, pub direction: Option<Direction>, pub ratio: Option<f64>,
    pub split_from: Option<String>, pub env: BTreeMap<String, String>,
}
pub enum PaneType { Agent, Command, Shell }
pub enum Direction { Right, Down }
pub struct Agent { pub name: String, pub label: Option<String>, pub icon: Option<String>,
    pub command: Vec<String>, pub extra_args: Vec<String>, pub options: Vec<AgentOption> }
pub struct AgentOption { pub name: String, pub args: Vec<String>, pub command: Option<Vec<String>> }

// on Config
pub fn layout(&self, name: &str) -> Option<&TabLayout>
pub fn agent(&self, name: &str) -> Option<&Agent>
pub fn render_label(icon: Option<&str>, label: &str) -> String  // "{icon}  {label}", exactly two spaces
```

Guaranteed by load-time validation, so **do not re-check any of it here**:

- `panes[0]` is the root and is `PaneType::Agent`; there is exactly one agent pane.
- Every non-root pane has a `direction` and a `ratio`, with `ratio` strictly between 0 and 1.
- `split_from`, when set, names an **earlier** pane in the same list, so a forward reference or a cycle is impossible.
- Pane names are unique within the layout.
- `command` is present for `PaneType::Command` and absent for the other two.

Because `direction` and `ratio` are `Option` in the struct but guaranteed present for non-root panes, unwrap them with `.expect("validated at load")` rather than inventing an error path the code cannot reach.

### New signature

```rust
fn build_side_panes(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    root_pane: &str,
    cwd: &str,
) -> Result<()>
```

### The loop

Skip `panes[0]` — the root already exists and the caller owns it. Maintain a `BTreeMap<&str, String>` from pane name to pane id, seeded with the root pane's name → `root_pane`. Track the previous pane's id separately for the default target.

For each remaining pane, in list order:

1. **Resolve the split target.** `split_from` names an earlier pane → look it up in the map. Otherwise the target is the pane directly above in the list — which is what a straight chain of splits wants, and what both example layouts use.

2. **`pane.split`** with `target_pane_id`, `direction` (`"right"` or `"down"`), `ratio: 1.0 - pane.ratio`, `cwd`, `focus: false`. Include `env` **only when the pane's `env` map is non-empty** — today's second split sends no `env` key at all, and adding an empty object would change the wire payload.

3. **Empty pane id is an error, not a silent skip.** Keep today's check and keep the failing pane named in the message:
   ```rust
   if pane_id.is_empty() {
       return Err(anyhow!("pane.split returned an empty pane id for pane {}", pane.name));
   }
   ```
   Record the new id in the map under this pane's name.

4. **`pane.rename`** with `render_label(pane.icon.as_deref(), label)` — **only when the pane sets a `label`**. A pane with no `label` keeps Herdr's own title, so no rename call is made at all. This is a real observable difference: a label-less pane must produce zero `pane.rename` calls, not a rename to an empty string.

5. **`pane.send_input`** with `text` = the pane's `command` line and `keys: ["enter"]` — **only for `PaneType::Command`**. `Shell` and `Agent` panes send nothing.

### The `1 - ratio` conversion is the load-bearing detail

A pane's `ratio` in config is **that pane's own share** of the split. Herdr's `ratio` is the **original** pane's share. Hence `1.0 - ratio`.

Concretely, from the default layout:

- `files` has `ratio = 0.62` → the Files column takes 62% of the width, so the agent pane keeps 38% → Herdr receives `0.38`, matching today's literal at `src/flows/agent.rs:183`.
- `term` has `ratio = 0.1` → the terminal strip takes 10% of the Files column's height, so Files keeps 90% → Herdr receives `0.9`, matching today's literal at `:204`.

Confirmed against a live session: the agent pane is the narrow one. The self-describing form is what makes a thin bottom strip read `0.1` rather than `0.9`, which is why the conversion exists at all. Put that reason in a comment — it is exactly the kind of inversion a later reader would "fix" backwards.

Compare ratios in tests with a tolerance (`< 1e-9`), never with `==` on `f64`.

### The root pane stays with the callers

`build_side_panes` never touches the root. Two call sites do:

- **`apply_launch_layout`** (`src/flows/agent.rs:168`) renames the root to the chosen label, then calls this function with the new signature. The comment above the call — splitting later keeps menus full-width and lets the chosen worktree drive every pane's cwd — still holds and must stay.
- **`build_popup_tab`** (`:360`) calls it with the new signature after renaming the root and the tab.

**`create_popup_tab` currently hardcodes the root's environment** at `:317`:

```rust
("env".to_owned(), json!({ "Q_NO_BANNER": "1" })),
```

That must now come from the layout's root pane `env`. Pass the layout down to `create_popup_tab` and serialize `layout.panes[0].env`. When the root pane's `env` is empty, omit the key entirely rather than sending `{}` — same rule as the split call. The default layout sets `Q_NO_BANNER = "1"` on its root pane, so the shipped wire payload is unchanged.

### Tests — the deliverable

The socket-sequence assertion is the only way to prove layout correctness without opening a live Herdr. Use `FakeClient`, which records every `(method, params)` pair.

1. **Default layout reproduces today's calls, field for field.** Replay the default three-pane layout and compare the **entire** recorded sequence, in order, against:

   | # | method | params |
   |---|---|---|
   | 1 | `pane.split` | `target_pane_id` = root, `direction: "right"`, `ratio: 0.38`, `cwd`, `env: {"Q_NO_BANNER": "1"}`, `focus: false` |
   | 2 | `pane.rename` | files pane id, `label: "\u{f0968}  Files"` |
   | 3 | `pane.send_input` | files pane id, `text: "yazi ."`, `keys: ["enter"]` |
   | 4 | `pane.split` | `target_pane_id` = files pane, `direction: "down"`, `ratio: 0.9`, `cwd`, `focus: false`, **no `env` key** |
   | 5 | `pane.rename` | term pane id, `label: "\u{f489}  term"` |

   Assert the absence of `env` on call 4 explicitly — a present-but-empty object is a failure. Write the two labels as `\u{...}` escapes; never paste a Nerd Font glyph into a Rust source file.

2. **`split_from` branches back.** A four-pane layout where the fourth pane sets `split_from` to the **first** pane rather than the third. Assert the fourth `pane.split` carries the root's pane id as `target_pane_id`, proving the named pane wins over the pane above.

3. **A label-less pane produces no rename.** A layout whose second pane omits `label` records zero `pane.rename` calls for that pane.

4. **A shell pane sends no input.** A layout with a `PaneType::Shell` pane records zero `pane.send_input` calls for it.

5. **An empty pane id fails loudly.** Queue a `pane.split` response with an empty `pane_id` and assert the error message names the pane.

**The existing tests near `src/flows/agent.rs:1173-1188` and `:1298-1313`** assert `FILES_LABEL` / `TERM_LABEL` renames against the old constants and must be rewritten against the layout-derived labels. Preserve their surrounding intent: the popup path and the in-pane path make the same calls, and the in-pane path defers splitting until after the menus.

## Acceptance criteria

- [ ] `build_side_panes(client, layout, root_pane, cwd)` iterates `layout.panes[1..]` and makes no hardcoded call.
- [ ] `split_from` resolves to the named earlier pane; omitting it targets the pane directly above.
- [ ] Herdr receives `1.0 - pane.ratio`, and a comment explains that config ratios are self-describing.
- [ ] `env` is sent on `pane.split` only when the pane's `env` map is non-empty.
- [ ] A pane without a `label` produces no `pane.rename` call; a non-`Command` pane produces no `pane.send_input` call.
- [ ] An empty `pane_id` from `pane.split` is an error naming the pane.
- [ ] `create_popup_tab` sources the root pane's `env` from the layout instead of the hardcoded `{"Q_NO_BANNER": "1"}`, omitting the key when empty.
- [ ] The default layout's recorded call sequence matches today's five calls exactly, including the absent `env` on the second split.

## Verification

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `cargo test flows::agent::` — the five tests above are present and passing.
- [ ] `grep -n '0\.38\|0\.9\|Q_NO_BANNER' src/flows/agent.rs` shows those literals only inside tests, never in the pane-building path.
- [ ] Run `git status --short` and quote it. Expect `src/flows/agent.rs`, plus at most this task file. Any OTHER path is a real scope violation.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Ratio sent uninverted, or the default layout's call sequence differs from today | Sequence matches but `env` is sent as `{}` on the second split, or a label-less pane still gets renamed | All five default calls byte-identical including absent `env`, `split_from` resolves to the named pane, empty pane id errors |
| Test coverage | ×2 | Only asserts that some calls happened | Default sequence checked loosely (method names only) | Whole sequence compared field for field with `f64` tolerance, plus `split_from`, label-less, shell-pane, and empty-id tests |
| Interface & readability | ×1 | Pane-id bookkeeping duplicated or the root special-cased inside the loop | Works but the name→id map and the previous-pane pointer are tangled | Root handled by callers, one map seeded with the root, loop body reads as split → check → rename → send |
| Assumptions & docs | ×1 | `1.0 - ratio` appears with no explanation | Mentioned but not justified | A comment states that config ratios are each pane's own share and Herdr's is the original pane's, with the `0.62 → 0.38` example |

## Out of scope

- Choosing which layout to build — this function receives a `&TabLayout`; resolving it from a flag or a default is separate work.
- The root pane's rename — it already belongs to the two callers and keeps that ownership.
- Restoring pane sizes after a manual resize, or evening out ratios — `src/flows/layout.rs` owns that and is unchanged.
- Supporting `left` / `up` split directions — Herdr's `pane.split` accepts `right` and `down` only.
