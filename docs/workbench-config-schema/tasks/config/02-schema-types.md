# CONFIG-02: Schema types for layouts and agents

> **Required reading** (read before starting; do not need to open other files):
> - `../_context/shared.md`
> - `../_context/rubric.md`
>
> **Depends on**: config/01
> **Blocks**: config/03
> **Status**: done

## Goal

`src/config.rs` carries serde types for `[[tab_layouts]]` and `[[agents]]`, the five flat model fields are gone, and the real `config.example.toml` parses through them with `deny_unknown_fields` on every struct.

## Files to create / modify

- `src/config.rs` (modify) — new types, five old fields deleted, two resolvers, one label helper, a parse test against the example file.
- `config.example.toml` (modify) — remove the `[[workspaces]]` section and repoint the `dashboard_workspace` comment.

## Implementation notes

`config.example.toml` at the repository root is the specification for this schema. **Read it before writing any code.** Every field below appears there with a comment explaining what it does. Where this task and that file disagree, the file wins — report the disagreement rather than guessing.

Keep the TOML field names exactly as the example writes them. The Rust identifiers may differ where Rust forbids the TOML spelling (`type` and `option` are the two cases).

### The types

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub dashboard_workspace: String,
    pub default_tab_layout: String,
    pub project_registry_file: String,
    pub projects_root: String,
    pub ssh_registry_file: String,
    pub ssh_config_file: String,
    pub ssh_history_file: String,
    pub tab_layouts: Vec<TabLayout>,
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabLayout {
    pub name: String,
    pub tab_label: Option<String>,
    // Defaulted, not required: a layout with no pane tables must deserialize to an empty
    // vec so validation can reject it by name, rather than serde reporting a generic
    // missing-field error that never says which layout.
    #[serde(default)]
    pub panes: Vec<LayoutPane>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPane {
    pub name: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    #[serde(rename = "type")]
    pub pane_type: PaneType,
    pub agent: Option<String>,
    #[serde(rename = "option")]
    pub option_name: Option<String>,
    pub command: Option<String>,
    pub direction: Option<Direction>,
    pub ratio: Option<f64>,
    pub split_from: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneType {
    Agent,
    Command,
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Right,
    Down,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    pub label: Option<String>,
    pub icon: Option<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub options: Vec<AgentOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOption {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub command: Option<Vec<String>>,
}
```

Notes on individual fields:

- `Config` loses `PartialEq, Eq` if `f64` enters it, because `f64` is not `Eq`. Derive `PartialEq` only, and check whether any existing test relies on `Eq` — `local_override_redirects_to_a_missing_file_without_error` compares two `Config` values with `assert_eq!`, which needs only `PartialEq`.
- `pane_type` renames to `type` because `type` is a Rust keyword.
- `option_name` renames to `option` because `Option` reads as the standard-library type at every use site. The TOML key stays `option`.
- A pane's `command` is a **single shell line** (`String`) — it is typed into the pane's interactive shell by `pane.send_input`. An agent's `command` is an **argv array** (`Vec<String>`) — every argument is its own entry, so `--cd /Users/q/My Projects` cannot split into two arguments. The two types are deliberately different; do not unify them.
- `env` defaults to an empty map rather than `Option`, because every consumer wants a map and an empty one is the correct no-op.

### Delete the five flat fields

Remove from both `Config` and `FileConfig`:

- `order: Vec<String>`
- `models: BTreeMap<String, String>`
- `model_args: BTreeMap<String, Vec<String>>`
- `claude_extra_args: Vec<String>`
- `codex_extra_args: Vec<String>`

Remove `apply_model_environment()` and `parse_environment_toml()` entirely, along with the five environment variables they read: `Q_AGENT_MODEL_ORDER`, `Q_AGENT_MODELS`, `Q_AGENT_MODEL_ARGS`, `Q_CLAUDE_EXTRA_ARGS`, `Q_CODEX_EXTRA_ARGS`. Remove those five names from the `names` array in the test harness `TestEnvironment::new()` so the harness stops clearing variables that no longer mean anything.

`resolve_args()` exists only for the two extra-args fields; delete it too if nothing else calls it.

`Config::load()` currently ends with three `..._from_file` booleans feeding `apply_model_environment`. Those go with it. The environment layer that **stays** is `resolve_string()` over the path scalars and `Q_DASHBOARD_WORKSPACE`, plus `expand_home()`.

This task does not need to make `Config::load()` produce sensible defaults for `tab_layouts` and `agents` — supplying the built-in defaults is separate work. Use `file.tab_layouts.unwrap_or_default()` and `file.agents.unwrap_or_default()` here, and `default_tab_layout` may resolve to `"agentic-coding"` as a plain string literal. Leave `Config::validate()` as a stub returning `Ok(())`; the validation rules land later.

### The temporary bridge — required, fully specified, do not improvise

Deleting the five fields breaks three call sites outside `src/config.rs`. `cargo test` is this task's gate, so those sites must compile and keep passing **now**, without redesigning the menus. The concrete failure this bridge prevents: `cargo test` cannot run at all otherwise.

Add exactly one helper and rewrite the three sites against it. Nothing more.

```rust
impl Config {
    /// Bridge to the pre-schema call sites: the one agent that carries a model menu.
    ///
    /// Temporary. The launch flow resolves its agent from the layout instead, and this
    /// goes away with the last caller.
    pub(crate) fn menu_agent(&self) -> Option<&Agent> {
        self.agents.iter().find(|agent| !agent.options.is_empty())
    }
}
```

**Site 1 — `src/flows/agent.rs`, the model menu (currently line 546).** `&config.order` becomes the option names of `menu_agent()`, in order:

```rust
let options: Vec<String> = config
    .menu_agent()
    .map(|agent| agent.options.iter().map(|o| o.name.clone()).collect())
    .unwrap_or_default();
let Some(model_label) = menu.choose(MODEL_TITLE, "Choose a model.", &options, 6)? else { ... };
```

**Site 2 — `src/flows/agent.rs`, `build_launch()` (currently lines 616-643).** Replace the body with a lookup against `[[agents]]`, keeping the existing substring dispatch on the harness label. For codex and opencode: find the agent whose `name` the harness label contains, and return its `command` followed by its `extra_args`. For claude: find the option in `menu_agent()` whose `name` equals the label, then return the option's `command` override when it has one, else `menu_agent()`'s `command` followed by the option's `args` and the agent's `extra_args`. The `if model == "CCR"` branch disappears here, because CCR's `command` override already does that job.

**Site 3 — `src/state.rs:70-77`, `last_choice_is_valid`.** The claude arm's `config.order.iter().any(...) && config.models.contains_key(...)` becomes: the model label matches an option name in `menu_agent()`. The `HARNESS_*` constants and the codex/opencode arms stay untouched.

The existing tests at `src/flows/agent.rs` around lines 1428-1441 drive `config.codex_extra_args` directly. Rewrite them to build a `Config` carrying `[[agents]]` instead. Their assertions — the argv they expect — must not change.

**Do not** add compatibility fields, a `Deref`, or a conversion trait. Two of these three sites are rewritten again in later work; a bridge that is a single method and three inlined lookups is trivially deletable, and a compatibility layer is not.

### `FileConfig`

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    dashboard_workspace: Option<String>,
    default_tab_layout: Option<String>,
    project_registry_file: Option<String>,
    projects_root: Option<String>,
    ssh_registry_file: Option<String>,
    ssh_config_file: Option<String>,
    ssh_history_file: Option<String>,
    tab_layouts: Option<Vec<TabLayout>>,
    agents: Option<Vec<Agent>>,
}
```

`deny_unknown_fields` on every struct is the point of this task, not a decoration. Without it a mistyped `direciton = "right"` is silently ignored and the pane is built wrong with no error anywhere. With it the user gets a named field error at load time.

### The label helper

Three call sites need the same rendering, so write it once:

```rust
/// Icon and label joined by exactly two spaces, matching every existing menu label.
pub fn render_label(icon: Option<&str>, label: &str) -> String {
    match icon {
        Some(icon) => format!("{icon}  {label}"),
        None => label.to_owned(),
    }
}
```

Two spaces, not one. The existing code splits harness labels on a two-space separator and joins usage and branch the same way. A missing `icon` renders the label alone, with no leading whitespace.

### The resolvers

```rust
impl Config {
    pub fn layout(&self, name: &str) -> Option<&TabLayout> {
        self.tab_layouts.iter().find(|layout| layout.name == name)
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.name == name)
    }
}
```

Linear search over a handful of entries. Do not add a map.

### Edit `config.example.toml`

The example file still carries a `[[workspaces]]` section from an earlier design round. `deny_unknown_fields` makes it a hard parse error, so it must go in this task rather than later.

- Delete the whole `# -- Workspaces ---` block and both `[[workspaces]]` entries.
- Delete the `layout = "..."` lines with them.
- Keep the `dashboard_workspace` key. Change its comment from "Names a `[[workspaces]]` entry" to say it is the Herdr workspace label the dashboard tab opens in, matched against Herdr's own workspace list.
- Leave `default_tab_layout`, both `[[tab_layouts]]` entries, and all three `[[agents]]` entries untouched.

**The TOML swallow rule**: every key written after a `[[table]]` header belongs to that table. All scalar settings must stay above the first `[[tab_layouts]]` header. Moving one below it turns it into a table field and trips `deny_unknown_fields` with an error that does not name the real cause. The example file already warns about this at the top; keep that warning.

**Do not type or paste a Nerd Font glyph.** The example file's icons are already correct. If an edit must touch a line containing one, use the repo's `unicode-edit` skill rather than the ordinary edit path — the ordinary path silently drops plane-15 codepoints, and a bash heredoc corrupts them by dropping the fifth hex digit.

### The gate test

```rust
#[test]
fn the_example_config_parses_with_no_unknown_fields() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
    let contents = fs::read_to_string(&path).expect("read config.example.toml");
    let file: FileConfig = toml::from_str(&contents).expect("parse config.example.toml");

    let layouts = file.tab_layouts.expect("example defines tab_layouts");
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[0].name, "agentic-coding");
    assert_eq!(layouts[0].panes.len(), 3);
    assert_eq!(layouts[0].panes[0].pane_type, PaneType::Agent);
    assert!(layouts[0].tab_label.is_none());
    assert_eq!(layouts[1].tab_label.as_deref(), Some("Personal Assistant"));

    let agents = file.agents.expect("example defines agents");
    assert_eq!(agents.len(), 3);
    assert_eq!(agents[0].name, "claude code");
    assert_eq!(agents[0].options.len(), 4);
    assert_eq!(agents[0].options[2].name, "CCR");
    assert_eq!(
        agents[0].options[2].command.as_deref(),
        Some(["ccr".to_owned(), "code".to_owned()].as_slice())
    );
}
```

This test is the reason the example file is called the executable specification: it fails the moment the file and the types disagree.

Add one negative test proving `deny_unknown_fields` is wired:

```rust
#[test]
fn a_mistyped_pane_field_is_a_named_error() {
    let error = toml::from_str::<FileConfig>(
        r#"
[[tab_layouts]]
name = "x"
  [[tab_layouts.panes]]
  name = "agent"
  type = "agent"
  direciton = "right"
"#,
    )
    .expect_err("reject an unknown pane field");

    assert!(error.to_string().contains("direciton"), "{error}");
}
```

## Acceptance criteria

- [x] `TabLayout`, `LayoutPane`, `PaneType`, `Direction`, `Agent`, and `AgentOption` exist with the TOML field names the example file uses.
- [x] `deny_unknown_fields` is present on `FileConfig`, `TabLayout`, `LayoutPane`, `Agent`, and `AgentOption`.
- [x] `order`, `models`, `model_args`, `claude_extra_args`, and `codex_extra_args` no longer appear in `src/config.rs`.
- [x] `apply_model_environment`, `parse_environment_toml`, and the five environment variable names are gone, including from the test harness's clearing list.
- [x] `render_label(Some("\u{f15ce}"), "claude code")` returns `"\u{f15ce}  claude code"`, and `render_label(None, "term")` returns `"term"`.
- [x] `Config::layout()` and `Config::agent()` resolve by name and return `None` for an unknown name.
- [x] `config.example.toml` has no `[[workspaces]]` section, still has `dashboard_workspace`, and its icons are byte-identical to before the edit.
- [x] The example file parses through `FileConfig` with the assertions above passing.

## Verification

- [x] `cargo test` passes.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo test config::tests::the_example_config_parses_with_no_unknown_fields` passes — this is the gate.
- [x] `rg 'Q_AGENT_MODEL|Q_CLAUDE_EXTRA_ARGS|Q_CODEX_EXTRA_ARGS' src/` returns no matches.
- [x] `rg 'workspaces' config.example.toml` returns no matches.
- [x] Run `git status --short` and quote it. Expect `src/config.rs`, `config.example.toml`, `src/flows/agent.rs`, and `src/state.rs`, plus at most this task file. Any OTHER path is a real scope violation.
- [x] Run `git diff src/flows/agent.rs src/state.rs` and confirm every hunk belongs to one of the three bridge sites named above: the model menu's option list, the `build_launch` body, and the claude arm of `last_choice_is_valid`.
- [x] Confirm the icons survived: `python3 -c "import sys;[print(hex(ord(c)),end=' ') for c in open('config.example.toml').read() if ord(c)>0xE000]"` prints the same codepoints as `git show HEAD:config.example.toml` piped through the same check, allowing for the removed section.

## Eval rubric

> Scale and shared dimensions: see `../_context/rubric.md`. Each dimension 0–5; weighted average > 4.0 to pass; Correctness < 4 is an automatic veto.

| Dimension | Weight | 0–1 (fail) | 2–3 (below bar) | 4–5 (pass) |
|---|---|---|---|---|
| Correctness | ×3 | A TOML field name diverges from the example file, or a Nerd Font glyph was corrupted | Types parse but `deny_unknown_fields` is missing somewhere, or pane `command` was typed as `Vec<String>` | Every field name matches the example, both `command` types stay distinct, glyphs byte-identical |
| Test coverage | ×2 | No test parses the real example file | Parses the file but asserts only that it succeeded | Asserts layout count, pane types, option count, the CCR command override, plus a negative unknown-field test |
| Interface & readability | ×1 | Adds a lookup map or a builder for a handful of entries | Resolvers duplicated at call sites instead of on `Config` | Linear resolvers on `Config`, one shared `render_label`, no speculative structure |
| Assumptions & docs | ×1 | Silently "fixes" a disagreement between this task and the example file | Renames a field without a comment saying why | `type`/`option` renames carry a one-line why; a disagreement with the example file is reported, not guessed at |

## Out of scope

- **Built-in defaults** — `unwrap_or_default()` is correct for now. Populating real defaults is separate work in this same bucket.
- **Validation** — `Config::validate()` stays a stub. The referential and structural rules land in later work in this bucket.
- **Redesigning the menus, the argv builder, or the state record** — the three bridge sites specified above are the *only* changes permitted in `src/flows/agent.rs` and `src/state.rs`. Do not thread a `&TabLayout` anywhere, do not change `AgentChoice`, do not touch `STATE_VERSION`. Those are later work and they delete the bridge as they go.
- **CHANGELOG and `CLAUDE.md`** — documentation is rewritten once, later.
