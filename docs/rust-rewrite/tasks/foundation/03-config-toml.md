# FOUNDATION-03: TOML configuration

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/parity.md`
>
> **Depends on**: foundation/01
> **Blocks**: foundation/04, registry/01, registry/04, agent/01
> **Status**: todo

## Goal

One typed config struct that resolves every setting the zsh version had, with
precedence user file → environment → built-in default, and no way for two call sites
to disagree about a default.

## Files to create / modify

- `src/config.rs` (new) — the `Config` struct, defaults, and resolution
- `config.example.toml` (new) — every setting documented, fully commented out

## Implementation notes

### Resolution order

The zsh version achieved user-file-wins by sourcing the user file *first*, so its
plain assignments survived the `:-` fallbacks. In Rust, resolve explicitly per field:

1. the value from the user's `config.toml`, if present
2. otherwise the environment variable, if set and non-empty
3. otherwise the built-in default

The path is
`${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench/config.toml`,
used as a literal. `Q_WORKBENCH_LOCAL_CONFIG` overrides it. A missing file is not an
error — it means "all defaults".

### The settings

Every default and the model-menu tables are inlined in the parity contract. Keep the
environment-variable names exactly as they are so an existing shell export still
works.

The two extra-args settings change type: `Vec<String>` instead of a space-split
string. In TOML:

```toml
claude_extra_args = ["--dangerously-skip-permissions"]
codex_extra_args  = ["--search", "--profile", "work"]
```

When the value arrives from the **environment** rather than the file, it is still a
single string and must be split on whitespace, preserving the old behaviour for that
channel. Document that an argument containing a space can only be expressed in the
file.

Both default to empty. **The sandbox and approval bypass flags stay opt-in** — do not
add a dedicated boolean, and do not default either list to anything.

### Model menu

Three related settings: an ordered list of labels, a label → model-value map, and a
label → extra-flags map. Represent them as `Vec<String>` and two
`BTreeMap<String, …>`; `BTreeMap` keeps the emitted TOML stable for diffing.

The zsh version had a trap here worth understanding even though it cannot recur: a
user file that assigned the maps as plain arrays before `typeset -gA` ran caused zsh
to silently empty them, so the user's labels resolved to nothing. In Rust the failure
mode to guard instead is **a label in the order list with no entry in the model map**.
Validate that at load time and return an error naming the offending label, rather than
producing a menu entry that selects nothing.

### `config.example.toml`

Mirrors what `config.example.zsh` did: every setting present, fully commented out,
with the default shown and a one-line explanation. Include an explicit note that the
bypass flags are opt-in and what they do.

## Acceptance criteria

- [ ] Every setting listed in the parity contract resolves to its documented default
      with no config file and no environment.
- [ ] An environment variable overrides the built-in default.
- [ ] A value in the user file overrides the environment, including overriding it back
      to empty.
- [ ] `Q_WORKBENCH_LOCAL_CONFIG` redirects the config path; a nonexistent path yields
      all defaults without error.
- [ ] Extra-args are `Vec<String>`; a file value may contain spaces inside one
      argument, an environment value is whitespace-split.
- [ ] Both extra-args lists default to empty — no bypass flag is ever added implicitly.
- [ ] A model-order label with no entry in the model map is a load error naming the label.
- [ ] `config.example.toml` documents every setting, fully commented out.

## Verification

- [ ] `cargo test` — one test per acceptance criterion above, mirroring the assertions
      in the existing `tests/config.test.zsh`
- [ ] A test asserts the resolved default path is
      `<home>/.config/herdr/plugins/config/q.workbench/config.toml` with
      `XDG_CONFIG_HOME` unset
- [ ] A test asserts a user file can clear an environment-set extra-args list back to empty
- [ ] `cargo clippy -- -D warnings` is clean

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A default differs from the zsh original, or precedence is inverted | All defaults right but the "file clears an env value" case fails | Every default and every precedence case matches |
| Test coverage | ×2 | No config tests | Defaults tested, precedence not | Defaults, both override directions, path resolution, and the invalid-model-label error all tested |
| Interface & readability | ×1 | Settings read ad hoc from several places | One struct but fields named inconsistently with the env vars | One struct, one loader, field names traceable to their env vars |
| Assumptions & docs | ×1 | `config.example.toml` missing or partial | Present but undocumented settings | Complete, commented, with the opt-in bypass note carried over |

## Out of scope

- Converting an existing `config.zsh` — that is the migration task.
- Reading the old `config.zsh` format at runtime. There is no permanent zsh parser.
