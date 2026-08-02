# CONFIG-03: Built-in defaults reproduce today

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: config/02
> **Blocks**: config/04
> **Status**: todo

## Goal

With no config file present, `Config::load()` yields the three harnesses, four claude options, and three-pane layout the plugin ships today — and a user who writes one `[[agents]]` entry gets exactly that one entry, not four.

## Files to create / modify

- `src/config.rs` (modify) — `default_agents()`, `default_tab_layouts()`, whole-section replacement in `load()`, and the tests that pin them.
- `src/state.rs` (modify) — only if it constructs `Config::default()` in tests.
- `src/flows/agent.rs` (modify) — only if it constructs `Config::default()` in tests.

## Implementation notes

The plugin must keep working for someone who clones the repo and runs it with no config file. That means the built-in defaults are not placeholders — they are the current shipping behaviour expressed in the new schema.

### `default_agents()`

Order matters: this is the harness menu order, top to bottom.

```rust
fn default_agents() -> Vec<Agent> {
    vec![
        Agent {
            name: "claude code".to_owned(),
            label: None,
            icon: Some("\u{f15ce}".to_owned()),
            command: vec!["claude".to_owned()],
            extra_args: Vec::new(),
            options: vec![
                AgentOption {
                    name: "Opus".to_owned(),
                    args: vec!["--model".to_owned(), "claude-opus-4-8".to_owned()],
                    command: None,
                },
                AgentOption {
                    name: "OpusPlan (Sonnet)".to_owned(),
                    args: vec![
                        "--model".to_owned(),
                        "opusplan".to_owned(),
                        "--effort".to_owned(),
                        "medium".to_owned(),
                    ],
                    command: None,
                },
                AgentOption {
                    name: "CCR".to_owned(),
                    args: Vec::new(),
                    command: Some(vec!["ccr".to_owned(), "code".to_owned()]),
                },
                AgentOption {
                    name: "Fable 5".to_owned(),
                    args: vec!["--model".to_owned(), "claude-fable-5".to_owned()],
                    command: None,
                },
            ],
        },
        Agent {
            name: "codex".to_owned(),
            label: None,
            icon: Some("\u{ee0d}".to_owned()),
            command: vec!["codex".to_owned()],
            extra_args: Vec::new(),
            options: Vec::new(),
        },
        Agent {
            name: "opencode".to_owned(),
            label: None,
            icon: Some("\u{f169f}".to_owned()),
            command: vec!["opencode".to_owned()],
            extra_args: Vec::new(),
            options: Vec::new(),
        },
    ]
}
```

Points that are easy to get wrong:

- **Write the icons as `\u{...}` escapes.** Never paste the glyph. Plane-15 codepoints get silently mangled by ordinary editing and by heredocs.
- `label` is `None` on all three. It defaults to `name`, and the rendered label is `format!("{icon}  {label}")`, which reproduces `"\u{f15ce}  claude code"` exactly.
- **CCR carries a `command` override and no `args`.** Today it is a hardcoded special case that dispatches to its own binary with no model flag and none of the claude extra args. Expressing it as an option with `command: Some(vec!["ccr", "code"])` is what lets that special case be deleted later. An option's `command` replaces the agent's `command` for that option alone.
- `codex` and `opencode` have no options, so they will skip the model menu.
- `extra_args` is empty on all three, and stays empty. **Nothing adds a bypass flag for the user.** `--dangerously-skip-permissions` and `--dangerously-bypass-approvals-and-sandbox` are opt-in, written by hand, per machine. An existing test asserts this; keep an equivalent.

### `default_tab_layouts()`

One layout, reproducing today's popup. It pins nothing, so all three menus run.

```rust
fn default_tab_layouts() -> Vec<TabLayout> {
    vec![TabLayout {
        name: "agentic-coding".to_owned(),
        tab_label: None,
        panes: vec![
            LayoutPane {
                name: "agent".to_owned(),
                label: None,
                icon: None,
                pane_type: PaneType::Agent,
                agent: None,
                option_name: None,
                command: None,
                direction: None,
                ratio: None,
                split_from: None,
                env: BTreeMap::from([("Q_NO_BANNER".to_owned(), "1".to_owned())]),
            },
            LayoutPane {
                name: "files".to_owned(),
                label: Some("Files".to_owned()),
                icon: Some("\u{f0968}".to_owned()),
                pane_type: PaneType::Command,
                agent: None,
                option_name: None,
                command: Some("yazi .".to_owned()),
                direction: Some(Direction::Right),
                ratio: Some(0.62),
                split_from: None,
                env: BTreeMap::from([("Q_NO_BANNER".to_owned(), "1".to_owned())]),
            },
            LayoutPane {
                name: "term".to_owned(),
                label: Some("term".to_owned()),
                icon: Some("\u{f489}".to_owned()),
                pane_type: PaneType::Shell,
                agent: None,
                option_name: None,
                command: None,
                direction: Some(Direction::Down),
                ratio: Some(0.1),
                split_from: None,
                env: BTreeMap::new(),
            },
        ],
    }]
}
```

Points that are easy to get wrong:

- **The root pane carries no geometry.** It fills the tab, so `direction`, `ratio`, and `split_from` are all `None`.
- **The root pane does carry `Q_NO_BANNER`.** Today `tab.create` sets it on the root, and the first split sets it on the files pane. The term pane gets none.
- **`ratio` is each pane's own share.** `files = 0.62` means the Files column takes 62% and the agent pane keeps 38%. `term = 0.1` means the terminal strip takes 10%. The loader converts to Herdr's value with `1 - ratio`, producing today's `0.38` and `0.9`. Do not "fix" these to look like the socket values.
- **`split_from` is `None` on both non-root panes.** It defaults to the pane directly above, which is exactly what this straight chain wants: `files` splits from `agent`, `term` splits from `files`.

`default_tab_layout` (the scalar pointer) defaults to `"agentic-coding"`.

### Whole-section replacement

```rust
tab_layouts: file.tab_layouts.unwrap_or_else(default_tab_layouts),
agents: file.agents.unwrap_or_else(default_agents),
```

A user who writes any `[[agents]]` entry replaces the built-in list entirely. This is deliberate and it matches how the old `order` / `models` / `model_args` fields behaved — `file.order.unwrap_or_else(default_order)`. The alternative, merging by name, would make "I only want codex" impossible to express. Put that reason in a comment; it is the kind of decision a future reader will otherwise try to "improve".

### The `Default` derive problem

`Config` currently carries:

```rust
#[cfg_attr(test, derive(Default))]
pub struct Config { ... }
```

A derived `Default` now produces **empty** `agents` and `tab_layouts`. Any test that builds a `Config` that way and then expects the real harnesses will fail in a way that looks like a logic bug rather than a fixture bug.

Drop the derive. Replace it with a test-only constructor built from the real default functions:

```rust
impl Config {
    #[cfg(test)]
    pub fn test_default() -> Self {
        Self {
            dashboard_workspace: "personal-assistant".to_owned(),
            default_tab_layout: "agentic-coding".to_owned(),
            project_registry_file: String::new(),
            projects_root: String::new(),
            ssh_registry_file: String::new(),
            ssh_config_file: String::new(),
            ssh_history_file: String::new(),
            tab_layouts: default_tab_layouts(),
            agents: default_agents(),
        }
    }
}
```

Known call sites to update:

- `src/state.rs` tests — `Config::default()` appears twice, in `round_trip_prunes_dead_panes_and_preserves_labels` and `stale_harness_is_removed`.
- `src/flows/agent.rs` tests — used to build a config for the `build_launch` tests.

Search for the rest rather than trusting this list.

### Tests

```rust
#[test]
fn missing_file_yields_the_shipping_harnesses_in_menu_order() {
    let _environment = TestEnvironment::new();
    let config = Config::load().expect("load defaults");

    let rendered: Vec<String> = config
        .agents
        .iter()
        .map(|agent| {
            render_label(agent.icon.as_deref(), agent.label.as_deref().unwrap_or(&agent.name))
        })
        .collect();

    assert_eq!(
        rendered,
        [
            "\u{f15ce}  claude code",
            "\u{ee0d}  codex",
            "\u{f169f}  opencode",
        ]
    );
}
```

Byte-equality against those three literals is the point. "Looks like the right label" is not a test.

Also cover:

- The four claude option names, in order: `["Opus", "OpusPlan (Sonnet)", "CCR", "Fable 5"]`.
- CCR's `command` is `Some(["ccr", "code"])` and its `args` are empty.
- `codex` and `opencode` have zero options.
- Every default agent's `extra_args` is empty — the bypass-flag guard.
- A config file containing a single `[[agents]]` entry yields `config.agents.len() == 1`, proving replacement rather than merge. Same for `[[tab_layouts]]`.
- The default layout's three panes carry the exact names, icons, labels, pane types, directions, ratios, and env maps listed above. Assert the ratios as `0.62` and `0.1`, and assert the root pane's `direction` and `ratio` are both `None`.
- `default_tab_layout` resolves to `"agentic-coding"` and `config.layout("agentic-coding")` returns `Some`.

The existing `missing_file_resolves_every_documented_default` test asserts the path scalars and the three deleted model fields. Keep its path assertions; drop the model ones and let the new tests above cover the new sections.

## Acceptance criteria

- [ ] `Config::load()` with no config file yields three agents named `claude code`, `codex`, `opencode`, in that order.
- [ ] Their rendered labels equal `"\u{f15ce}  claude code"`, `"\u{ee0d}  codex"`, `"\u{f169f}  opencode"` byte for byte.
- [ ] The claude agent has four options in the order `Opus`, `OpusPlan (Sonnet)`, `CCR`, `Fable 5`; `CCR` carries `command = Some(["ccr", "code"])` and empty `args`.
- [ ] Every default agent's `extra_args` is empty — no implicit bypass flag.
- [ ] The default layout is named `agentic-coding`, has `tab_label: None`, and three panes matching the names, icons, labels, types, directions, ratios, and env maps specified above.
- [ ] The root pane has `direction: None`, `ratio: None`, `split_from: None`, and `env` containing `Q_NO_BANNER = "1"`.
- [ ] A config file with one `[[agents]]` entry yields exactly one agent; a config file with one `[[tab_layouts]]` entry yields exactly one layout.
- [ ] `Config` no longer derives `Default`; every former `Config::default()` call site builds a config that carries the real defaults.
- [ ] All icons in `src/config.rs` are written as `\u{...}` escapes; no literal glyph appears in the file.

## Verification

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `cargo test config::` passes, including the byte-equality label test.
- [ ] `rg 'Config::default\(\)' src/` returns no matches.
- [ ] `rg -n 'icon' src/config.rs` shows only `\u{...}` escapes on the default-agent and default-layout lines, never a raw glyph.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A ratio written as the socket value (`0.38`/`0.9`), or a glyph pasted instead of escaped, or CCR given `args` instead of a `command` override | Defaults present but menu order wrong, or `Q_NO_BANNER` missing from the root pane | Every default byte-matches today's behaviour, ratios expressed as each pane's own share, CCR is a `command` override |
| Test coverage | ×2 | No test compares rendered labels to the literal constants | Asserts names but not the rendered `${icon}  ${label}` strings | Byte-equality on all three labels, option order pinned, replacement-not-merge proven for both sections, bypass-flag guard kept |
| Interface & readability | ×1 | Defaults built by parsing an embedded TOML string at runtime | Two default functions duplicating the same literals | Two plain `Vec` constructors, one `test_default()`, no cleverness |
| Assumptions & docs | ×1 | Whole-section replacement left unexplained, so a future reader "fixes" it into a merge | Ratio semantics undocumented at the default site | A comment names why replacement beats merge, and why `0.62` is not `0.38` |

## Out of scope

- **Validation** — nothing here rejects a bad config. The validation rules land in the next piece of work in this bucket. A default config is valid by construction, so no validation is needed to make these tests pass.
- **The second example layout** — `config.example.toml` ships a `personal-assistant` layout as a contrast case. It is not a built-in default and must not appear in `default_tab_layouts()`.
- **Reaching a non-default layout** — the `--layout` flag is separate work in the wiring bucket. Here, `default_tab_layout` is just a string on `Config`.
- **`src/flows/` behaviour** — if the launch flow needs edits to compile against the new defaults, keep them mechanical.
