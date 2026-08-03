# Architecture context

> The current shapes tasks in this plan read or change. Signatures are copied from the
> repository so an executor does not have to open the files first — but the files are the
> authority if they have drifted.

## Target flow

```
tab new
  └─ Config::load()                     config errors before the flow's first socket call
  └─ terminal viewport                  agent::popup_viewport(), same source as the popup
  └─ tab::choose_layout(config, menu)   skipped when exactly one layout is configured
  └─ agent::popup_with_layout(client, &config, layout, worktree: false)
       └─ adopt_invoking_pane_cwd()     popup cwd is the plugin checkout, not the project
       └─ choose_agent() → realise_worktree() → create_popup_tab()
```

**Scope of the ordering invariant.** `main` builds the `SocketClient` and runs the
protocol guard (`ping`) before dispatching any command whose `Channel` is
`Notification` — that is true of every existing action today. The invariant a flow owns is
narrower: the config is loaded and validated before *that flow* issues its first request,
so a broken config never leaves a half-built tab on screen.

## `src/config.rs`

### Current `TabLayout`

```rust
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
```

`deny_unknown_fields` is on, so an unknown key in `[[tab_layouts]]` is already a load
error. Adding a field is backwards compatible; removing one is not.

### The rendering helper both menus use

```rust
/// Icon and label joined by exactly two spaces, matching every existing menu label.
/// A missing icon renders the label alone, with no leading whitespace.
pub fn render_label(icon: Option<&str>, label: &str) -> String {
    match icon {
        Some(icon) => format!("{icon}  {label}"),
        None => label.to_owned(),
    }
}
```

### The pattern to mirror for layouts

```rust
impl Agent {
    /// The harness menu row for this agent. The reverse lookup in the harness menu matches
    /// on this exact string, so every site that renders an agent must go through here.
    pub fn menu_label(&self) -> String {
        render_label(
            self.icon.as_deref(),
            self.label.as_deref().unwrap_or(&self.name),
        )
    }
}
```

And the matching validation inside `Config::validate()`:

```rust
// The harness menu returns the rendered row and maps it back to an agent by
// that string. Two agents rendering the same row would both be listed while
// only the first could ever be selected.
let label = agent.menu_label();
if let Some(other) = agent_labels.insert(label.clone(), agent.name.as_str()) {
    bail!(
        "agents '{}' and '{}' render the same menu label: {}",
        other, agent.name, label
    );
}
```

`Config::validate()` already walks `self.tab_layouts` twice: once collecting
`layout_names` and rejecting duplicates, once checking each layout's panes. Every error is
a `bail!` naming the offending layout.

### Lookups

```rust
impl Config {
    pub fn layout(&self, name: &str) -> Option<&TabLayout>;   // find by `name`
    pub fn agent(&self, name: &str) -> Option<&Agent>;
}
```

`Config` also carries `pub default_tab_layout: String`, validated at load to name an
existing layout, and `pub tab_layouts: Vec<TabLayout>` in config order.

Two tests pin the config surface and will need rows when fields are added:
`the_example_config_parses_with_no_unknown_fields` (parses `config.example.toml`) and the
`default_tab_layouts()` equality test.

## `src/flows/agent.rs`

### The menu primitives (to be moved to `src/flows/menu.rs`)

```rust
/// One menu step. `Ok(None)` means the user cancelled, which is normal and quiet.
trait Menu {
    fn choose(&mut self, title: &str, subtitle: &str, options: &[String], height: u8)
        -> Result<Option<String>>;
    fn filter(&mut self, title: &str, subtitle: &str, options: &[String], placeholder: &str)
        -> Result<Option<String>>;
    fn input(&mut self, title: &str, subtitle: &str, placeholder: &str, width: u16,
        indent: InputIndent) -> Result<Option<String>>;
}

enum InputIndent { Centered, None }

struct GumMenu { cols: u16, lines: u16 }
impl GumMenu {
    fn new(cols: u16, lines: u16) -> Self;
    fn content_width(&self) -> u16;      // 44, narrowed on a small viewport
    fn content_margin(&self) -> u16;
    fn block_margin(&self, width: u16) -> u16;
    fn vertical_padding(&self, height: u16) -> u16;
    fn render_banner(&self, title: &str, subtitle: &str, body_lines: u16) -> Result<()>;
    fn padded(&self, options: &[String]) -> Vec<String>;
}
impl Menu for GumMenu { … }

fn gum_output<I, S>(args: I) -> Result<Option<String>> where I: IntoIterator<Item = S>, S: AsRef<OsStr>;
fn gum_with_input(args: &[&str], input: &str) -> Result<Option<String>>;
fn strip_pad(value: &str) -> String;     // value.trim_start().to_owned()
fn display_width(value: &str) -> u16;    // East-Asian-wide aware column count
const FILTER_HEIGHT_ARG: &str = "12";
const FILTER_HEIGHT: u16 = 12;
```

Behaviour worth preserving verbatim:

- `gum` draws its UI on **stderr** when stdout is not a terminal, which is always the case
  here because the selection is captured. `gum_output` inherits stderr for that reason.
- A non-zero `gum` exit means cancellation: `Ok(None)`, never an error.
- `render_banner` prints the banner line by line at a computed margin. Wrapping an
  already-styled banner in a second `gum style` renders it ragged.
- `padded` indents every option by one shared margin so glyphs line up in one column;
  `gum choose` runs with an empty `--cursor` so it adds no prefix of its own.
- `strip_pad` removes the leading pad but keeps the Nerd Font glyph.

### The menu-consuming call sites in `agent.rs`

`choose_agent` / `choose_agent_with_last` / `select_usage` / `select_worktree` take
`menu: &mut impl Menu`. `choose_agent` constructs `GumMenu::new(cols, lines)` and delegates.
Tests in the `mod popup` block define a `FakeMenu` implementing `Menu` with a queued script
of answers.

### The harness menu's reverse lookup (the pattern `choose_layout` copies)

```rust
let Some(harness) = menu.choose(HARNESS_TITLE, "Choose a harness.", &harness_options, 8)? else {
    return Ok(None);
};
let harness = strip_pad(&harness);
if harness.is_empty() {
    return Ok(None);
}
// Rendered labels are unique across agents, enforced at config load.
let agent_name = config
    .agents
    .iter()
    .find(|agent| agent.menu_label() == harness)
    .map(|agent| agent.name.clone())
    .expect("validated at load");
```

Note both cancellation shapes: `None` from the menu, and an empty string after
`strip_pad`.

### The popup entry point

```rust
/// Collect a popup decision, then create and focus its tab.
pub fn popup(client: &dyn HerdrClient, worktree: bool, requested_layout: Option<&str>) -> FlowResult {
    // Config first: adopting the invoking pane's cwd queries Herdr, and a broken config
    // must be reported before the first socket call.
    let config = Config::load().context("failed to load config")?;
    let layout = resolve_layout(&config, requested_layout)?;
    adopt_invoking_pane_cwd(client)?;
    let cwd = std::env::current_dir().context("failed to read popup working directory")?;
    let (cols, lines) = popup_viewport();
    let Some(mut choice) = choose_agent(&config, layout, &cwd, worktree, None, cols, lines, None)?
    else {
        return Ok(Outcome::Cancelled);
    };

    if let Some(branch) = choice.branch.clone() {
        let repo_root = RealGit.toplevel(&cwd).unwrap_or_else(|| cwd.clone());
        if realise_worktree(&repo_root, &branch).is_none() {
            choice = without_worktree(choice, &repo_root);
        }
    }

    create_popup_tab(client, layout, &choice, nonempty_env("HERDR_WORKSPACE_ID"))?;
    Ok(Outcome::Done)
}

fn adopt_invoking_pane_cwd(client: &dyn HerdrClient) -> Result<()>;
fn popup_viewport() -> (u16, u16);   // flows::terminal_size(), then COLUMNS/LINES, then tput, then 80
fn resolve_layout<'a>(config: &'a Config, requested: Option<&str>) -> Result<&'a TabLayout>;
```

`create_popup_tab` creates the tab, and on any failure after creation closes it and returns
`FlowError::prefixed("Agent tab failed", "The incomplete tab was closed.", error)`.

Existing popup tests to keep green: `popup_reproduces_the_exact_ten_call_sequence`,
`popup_workspace_id_is_omitted_when_empty_and_sent_when_present`,
`popup_cwd_prefers_plugin_context_and_falls_back_to_active_pane`,
`popup_failure_at_every_post_create_step_closes_and_returns_metadata`,
`popup_cancelled_choice_makes_zero_calls`,
`popup_extra_args_preserve_toml_array_boundaries_and_bypass_is_opt_in`.

## `src/flows/mod.rs`

```rust
pub mod agent;
pub mod dashboard;
pub mod layout;
pub mod picker;
pub mod restart;
pub mod ssh;

pub fn nonempty_env(name: &str) -> Option<String>;
pub fn terminal_size() -> Option<(u16, u16)>;

pub enum Outcome { Done, Cancelled, Notice { title: String, body: String } }
pub type FlowResult = anyhow::Result<Outcome>;

pub struct FlowError { … }
impl FlowError {
    pub fn titled(title: impl Into<String>, error: anyhow::Error) -> Self;
    pub fn prefixed(title: impl Into<String>, prefix: impl Into<String>, error: anyhow::Error) -> Self;
    pub fn complete(title: impl Into<String>, body: impl Into<String>) -> Self;
}
```

## `src/main.rs`

The router is one `Cli` with a `Command` enum. Four things must stay in sync for every
leaf, and there is a test for each:

```rust
enum Command { Agent { .. }, Project { .. }, Ssh { .. }, Dashboard, Herdr { .. }, Pane { .. } }

enum Channel {
    Notification(&'static str),          // popup-facing: failures notify
    Stderr { uses_herdr: bool },         // terminal-facing: failures print
}

impl Cli {
    fn channel(&self) -> Channel;                 // exhaustive match, one arm per leaf
    fn subcommand_path(&self) -> &'static str;    // e.g. "agent popup", used in stderr prefixes
    fn run(self, client: Option<&dyn HerdrClient>) -> FlowResult;
    fn uses_herdr(&self) -> bool;                 // derived from channel()
}
```

`Channel::Notification` implies `uses_herdr`, so `main` builds a `SocketClient` and runs the
protocol guard before `run`. The router arm gets the client via
`client.context("Herdr client is required for …")?`.

Tests that enumerate leaves and must gain a row for a new subcommand:
`every_leaf_parses_with_all_supported_arguments`,
`non_agent_notifying_subcommands_carry_their_contract_titles`,
`every_subcommand_selects_its_fixed_channel`.

## `herdr-plugin.toml`

An action that opens a popup pane has two halves — the action Herdr lists, and the pane it
opens:

```toml
[[actions]]
id = "new-agent"
title = "New agent"
contexts = ["workspace"]
command = ["herdr", "plugin", "pane", "open", "--plugin", "q.workbench",
           "--entrypoint", "agent", "--placement", "popup",
           "--width", "60%", "--height", "70%"]

[[panes]]
id = "agent"
title = "󱚟  new agent"
placement = "popup"
command = ["./bin/workbench", "agent", "popup"]
```

`--entrypoint` names the `[[panes]]` `id`. Pane titles carry a Nerd Font glyph followed by
two spaces.

## Testing helpers

- `workbench::herdr::FakeClient` — queues ordered responses and errors per method and
  records every call as `(method, params)`. `queue_response(method, json)` and
  `queue_error(method, code, message)`. Used by every flow test.
- `Config::test_default()` — a `Config` with the built-in layouts and agents, for tests
  that do not need a file on disk.
- Config tests that need a file write a TOML file to a temporary directory and point
  `Q_WORKBENCH_LOCAL_CONFIG` at it.
