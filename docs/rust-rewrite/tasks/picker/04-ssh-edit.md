# PICKER-04: `workbench ssh edit`

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: picker/03, registry/05
> **Blocks**: picker/05
> **Status**: done

## Goal

Add a host to the SSH config, or open the config for an existing one, from inside the
picker without corrupting the file.

## Files to create / modify

- `src/flows/ssh.rs` (modify) — the host editor. This module already exists; the session
  wrapper created it. Add to it rather than starting a second SSH module. That shared
  file is the only reason this task waits on the session task — there is no behavioural
  dependency between the editor and the session.
- `src/main.rs` (modify) — wire `ssh edit <target>`

## Implementation notes

### The target is optional

`ssh edit` takes an **optional** positional target. The picker's `ctrl-i` binding
passes `{2}`, which fzf expands to an empty string when nothing is selected, so a
required argument would have clap reject the call before this flow could say anything.

With no target — or an empty one — report `No SSH target selected.` on stderr and exit
1. Do not fall through to the add-host prompts with blank defaults.

### Two paths

- **Config-sourced target** — open the SSH config in `$VISUAL`, then `$EDITOR`, then
  `nvim`, and stop there. Nothing is written by this program.
- **Manual target** — prompt for alias, hostname, user and port, show the block that
  will be written, confirm, then append it.

Defaults for the prompts come from the target itself: a target shaped `user@hostname`
pre-fills both fields; otherwise the whole target is the hostname. Port defaults to
`22`.

### Screen ownership

This runs from an fzf `execute` binding, and fzf owns the alternate screen. `clear`
before drawing anything, so fzf can redraw correctly when the command exits. Skipping
this leaves the picker visually corrupted.

### Validation

All preserved exactly from the zsh version:

- alias matches `[A-Za-z0-9_.-]+` (whole string)
- hostname is non-empty and contains no whitespace
- user, when given, matches the same class as the alias
- port is numeric and in the range 1–65535
- an alias already present in the config is refused, matched case-insensitively on the
  `Host` keyword and exactly on the alias token

Make validation a pure function over a struct of the four fields, so tests call it
directly rather than driving prompts. The environment-variable override hooks the zsh
version used for testing (`Q_SSH_EDIT_ALIAS` and friends) are no longer needed and can
be dropped.

### Atomic write

Copy the config to a temp sibling preserving its mode, append the block, then rename
over the original. `trash` the temp file on any failure. The block is preceded by a
blank line when the existing file is non-empty. The `User` line is omitted entirely
when no user was given.

Never append to the config in place — a partial write to an SSH config locks the user
out of their own hosts.

After a successful write, run the registry's `sync` and then `use` for the target.

## Acceptance criteria

- [x] The target is an optional positional; an absent or empty one reports
      `No SSH target selected.` on stderr and exits 1 without prompting.
- [x] A config-sourced target opens the config in `$VISUAL`, `$EDITOR`, or `nvim`, in
      that order, and writes nothing.
- [x] A manual target prompts for all four fields with the documented defaults.
- [x] The screen is cleared before anything is drawn.
- [x] Every validation rule is enforced, and a duplicate alias is refused.
- [x] The written block matches the documented layout, with `User` omitted when empty
      and a leading blank line only for a non-empty file.
- [x] The write is atomic and leaves no temp file on any path.
- [x] A successful write runs the registry's `sync` then `use`.
- [x] Cancelling any prompt writes nothing and exits 0.

## Verification

- [x] `cargo test` — an absent target and an empty-string target both produce
      `No SSH target selected.` and exit 1, with no prompt spawned
      (`missing_and_empty_targets_fail_before_ui`, answered by a prompt that panics if
      asked)
- [x] `cargo test` — validation table with a valid case plus one failure per rule: bad
      alias, empty hostname, hostname containing a space, bad user, port 0, port 65536,
      non-numeric port, duplicate alias (`validates_every_ssh_config_field`, which also
      pins `+22` and ` 22`)
- [x] `cargo test` — the appended block's exact bytes for a host with a user and one
      without, against an empty file and a non-empty file
      (`renders_exact_block_bytes_for_all_file_and_user_combinations`)
- [x] `cargo test` — a simulated write failure leaves the original config unchanged and
      no temp file behind (`failed_replace_preserves_original_and_removes_temporary_file`)
- [x] `cargo test` — cancelling at each of the four inputs and at the confirm writes
      nothing, leaves no file behind and exits 0
      (`cancelling_any_prompt_writes_nothing_and_exits_zero`)
- [x] Live add against the real binary, driven through a pty by `expect` rather than the
      linked dev plugin: two hosts added to a sandbox config. The first wrote
      `Host myhost\n  HostName example.com\n  User alice\n  Port 22\n` to an empty file,
      the second appended after a blank line, `Added SSH config: <alias>` went to stdout
      with exit 0, the registry collapsed the manual `alice@example.com` entry into the
      new config entry with `last_used_at` stamped, and no temp file survived. The
      transcript starts with the `clear` sequence. **Not verified**: fzf's redraw, which
      needs a live pane.
- [x] Manual: a config-sourced host opens the editor and changes nothing — run with
      `EDITOR=/bin/echo`, which printed the config path and left the file byte-identical.
- [x] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | Writes in place, or a validation rule is dropped | Atomic but the block layout or the duplicate check differs | Every rule enforced, exact block bytes, atomic on every path |
| Test coverage | ×2 | No validation tests | Validation only | Validation table, block bytes for all four combinations, failure-leaves-original, plus the live add |
| Interface & readability | ×1 | Validation embedded in the prompt flow | Extracted but duplicated for tests | One pure validation function used by both the prompt path and the tests |
| Assumptions & docs | ×1 | No note on the `clear` requirement | Mentioned in passing | Explains fzf's alternate-screen ownership and why the write must be atomic |

## Out of scope

- Removing a host from the SSH config file. The registry's `remove` only hides
  config-sourced entries, as it does today.
- Editing an existing config entry's fields programmatically. That path opens an editor
  by design.
