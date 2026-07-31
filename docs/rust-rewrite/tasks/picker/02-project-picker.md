# PICKER-02: `workbench project pick`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: picker/01, agent/03
> **Blocks**: polish/03
> **Status**: done

## Goal

The project popup: fuzzy-pick a project, then focus its existing workspace or create
one — optionally with an agent tab already built.

## Files to create / modify

- `src/flows/picker.rs` (modify) — add the project picker
- `src/main.rs` (modify) — wire `project pick`

## Implementation notes

The full fzf argument list, key bindings, border label, and the enter-versus-alt-enter
behaviour are inlined in the parity contract.

### Self-referencing bindings

The `reload` and `execute` bindings call this same binary. Build them from
`std::env::current_exe()`, shell-quoted, never a hardcoded path — the binary's
location differs between the installed plugin and the dev harness.

The edit binding runs the registry's `edit` operation and then reloads the source.
Anything drawn by that binding must `clear` first: fzf owns the alternate screen and
needs it clean to redraw when the command exits.

### Result parsing

With `--print-query --expect=alt-enter`, fzf writes three lines: the query, the
pressed key (empty for plain enter), then the selected payload. Parse positionally;
all three lines are always present even when empty. A non-zero fzf exit means the user
cancelled — exit 0 with no side effect.

### Resolving the path

If nothing was selected but a query was typed: use the query as a directory if it
exists, otherwise `zoxide query -- <query>`. If neither yields an existing directory,
report the failure and exit non-zero. Resolve symlinks on whatever is chosen.

### Workspace focus or create

Find an existing workspace by scanning `session.snapshot` for a pane whose `cwd` or
`foreground_cwd` equals the resolved path; take the first match's `workspace_id` and
focus it. This is the "client-side filtering is the search" pattern.

Only when there is no match, create a workspace with the project's display name as its
label, `Q_NO_BANNER` set, and focus off.

One `workspace.create` call returns everything needed — its result carries `workspace`,
`tab` and `root_pane` objects, so read `workspace.workspace_id`, `tab.tab_id` and
`root_pane.pane_id` from that single response rather than making follow-up calls. The
full response shape is in the Herdr API context file. Then:

- **enter** — rename the new tab to the pinned main label from the parity contract and
  inject the agent launcher into its root pane with that same label as the fixed usage
- **alt-enter** — leave the workspace plain

Finally focus the workspace, then stamp the registry's `use` for the project.

Note the asymmetry, and keep it: an **existing** workspace is only focused — no agent
tab is built and the registry is still stamped.

## Acceptance criteria

- [x] The fzf invocation matches the parity contract: every flag, the prompt, the
      pointer, the border label, and all three bindings.
- [x] Bindings are built from the running executable's own path, shell-quoted.
- [x] Cancelling fzf exits 0 with no side effect.
- [x] A query with no selection falls back to a directory, then to zoxide; failing
      both, it reports and exits non-zero.
- [x] An existing workspace is focused; no tab is built; the registry is still stamped.
- [x] A new workspace gets the agent tab on enter and stays plain on alt-enter.
- [x] The workspace, tab and root-pane ids all come from the single `workspace.create`
      response; no follow-up lookup call is made to find them.
- [x] The new tab is renamed to the pinned main label before the launcher is injected.
- [x] A missing registry notifies with title `Project picker` and body
      `project picker: registry not found: <path>`, then exits 1; a query resolving to
      nothing notifies with body `project picker: project not found: <query or path>`
      and exits 1. Both are notifications, not stderr: this runs inside a popup pane,
      so its stderr is never seen. The text is preserved verbatim.

## Verification

- [x] `cargo test` — result parsing for all four combinations of query present/absent
      and key pressed/not
- [x] `cargo test` — binding strings contain the current executable path, correctly
      quoted for a path containing a space
- [x] `cargo test` — with `FakeClient`: existing-workspace path issues only
      `session.snapshot` and `workspace.focus`; new-workspace enter path issues
      `workspace.create`, `tab.rename`, the inject calls, then `workspace.focus`
- [x] `cargo test` — alt-enter on a new workspace issues no rename and no inject
- [x] Manual through the linked dev plugin: pick an existing project, pick an
      unregistered directory via zoxide, and use `ctrl-i` to edit an entry — confirm
      fzf redraws cleanly after the editor exits
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Hardcodes the binary path, or builds an agent tab for an existing workspace | fzf flags right but a fallback or the alt-enter branch differs | Every flag, binding, branch and asymmetry reproduced |
| Test coverage | ×2 | No parsing tests | Parsing only | Parsing, binding construction, both workspace branches, and both key paths |
| Interface & readability | ×1 | fzf argument list built inline as one string | Extracted but hard to compare against the original | Arguments listed one per line, directly comparable to the parity contract |
| Assumptions & docs | ×1 | No note on the `clear` requirement | Mentioned in passing | Explains that fzf owns the alternate screen and why the existing-workspace path differs |

## Out of scope

- Changing the enter/alt-enter semantics or the pinned tab label.
- The SSH picker — separate task, same fzf conventions.
