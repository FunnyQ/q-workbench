# AGENT-05: `workbench dashboard`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/herdr-api.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/03, foundation/05
> **Blocks**: polish/03
> **Status**: todo

## Goal

Open a dedicated tab in the configured workspace and start Claude there with the
dashboard prompt already submitted.

## Files to create / modify

- `src/flows/dashboard.rs` (new) — the dashboard launcher
- `src/main.rs` (modify) — wire `dashboard`

## Implementation notes

The workspace resolution rule, the tab label, the prompt string and the launch command
are all in the parity contract. Two points worth restating because they are easy to
get wrong:

- **Resolve the workspace by label on every invocation.** Herdr workspace ids are not
  durable, so a cached id is a latent bug. `workspace.list`, then find the entry whose
  `label` matches the configured dashboard workspace. If none matches, notify and exit
  non-zero — do not create one.
- **The tab is created focused**, unlike every other flow in this plugin.

The prompt is passed as an argument to `claude` rather than typed separately, so Claude
starts processing it immediately instead of leaving it staged. It contains a space and
a slash, so it must go through the shell-quoting helper before being embedded in the
`pane.send_input` text.

This is the smallest flow in the plugin and a good early smoke test of the socket
client plus the quoting helper together.

## Acceptance criteria

- [ ] The workspace is resolved by label on every run; a missing label notifies with
      title `Dashboard Launcher` and body `Workspace '<label>' was not found.` — the
      configured label interpolated inside single quotes — at position `bottom-right`,
      then exits non-zero.
- [ ] The tab is created with the parity contract's label, `Q_NO_BANNER` set, and
      focus on.
- [ ] The launch command is `claude --model sonnet <quoted prompt>` submitted with
      `keys: ["enter"]`.
- [ ] The prompt string matches the parity contract exactly.
- [ ] No workspace is ever created by this flow.

## Verification

- [ ] `cargo test` — with `FakeClient`, assert the full call sequence and every
      parameter for the success path
- [ ] `cargo test` — a `workspace.list` result with no matching label produces one
      `notification.show` and no `tab.create`
- [ ] `cargo test` — the submitted text round-trips through a shell back to exactly
      four argv elements: `claude`, `--model`, `sonnet`, and the prompt as one element
      (the prompt contains a space and must not split)
- [ ] Manual through the linked dev plugin: run it once and confirm Claude starts with
      the prompt already processing
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Caches a workspace id, creates a workspace, or mangles the prompt | Works but the tab is not focused or the label differs | Label-resolved every run, exact tab label and prompt, focused tab |
| Test coverage | ×2 | No tests | Success path only | Success, missing-workspace, and the quoting round-trip |
| Interface & readability | ×1 | Prompt and label inlined at several places | Constants defined but scattered | Constants at the top of the module, one call path |
| Assumptions & docs | ×1 | No note on id durability | Mentioned in passing | Explains why the workspace is resolved by label every time |

## Out of scope

- Making the prompt or the model configurable. It is a fixed personal shortcut.
