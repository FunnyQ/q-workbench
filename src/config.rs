use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub dashboard_workspace: String,
    pub default_tab_layout: String,
    pub project_registry_file: String,
    pub projects_root: String,
    pub project_markers: Vec<String>,
    pub ssh_registry_file: String,
    pub ssh_config_file: String,
    pub ssh_history_file: String,
    pub tab_layouts: Vec<TabLayout>,
    pub agents: Vec<Agent>,
}

impl Config {
    #[cfg(test)]
    pub fn test_default() -> Self {
        Self {
            dashboard_workspace: "personal-assistant".to_owned(),
            default_tab_layout: "agentic-coding".to_owned(),
            project_registry_file: String::new(),
            projects_root: String::new(),
            project_markers: default_project_markers(),
            ssh_registry_file: String::new(),
            ssh_config_file: String::new(),
            ssh_history_file: String::new(),
            tab_layouts: default_tab_layouts(),
            agents: default_agents(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    dashboard_workspace: Option<String>,
    default_tab_layout: Option<String>,
    project_registry_file: Option<String>,
    projects_root: Option<String>,
    project_markers: Option<Vec<String>>,
    ssh_registry_file: Option<String>,
    ssh_config_file: Option<String>,
    ssh_history_file: Option<String>,
    tab_layouts: Option<Vec<TabLayout>>,
    agents: Option<Vec<Agent>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabLayout {
    pub name: String,
    pub label: Option<String>,
    pub icon: Option<String>,
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
    // Rename: type is a Rust keyword.
    #[serde(rename = "type")]
    pub pane_type: PaneType,
    pub agent: Option<String>,
    // Rename: Option is a standard-library type; reads ambiguously at every use site.
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

/// File names that mark a directory as a project even without a `.git` in it.
///
/// The project picker's sweep treats one of these the way it treats `.git`: as a leaf,
/// which both admits the directory and stops the walk there. That is what separates a
/// project from the directory that merely holds projects — depth cannot, because a
/// projects root nests them unevenly.
fn default_project_markers() -> Vec<String> {
    ["package.json", "Gemfile", "Cargo.toml", "CLAUDE.md"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

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

/// The name of the built-in blank layout.
///
/// Reserved rather than private: a config may declare a layout under this name to change
/// what "blank" opens, and the menu shows that one instead. Either way exactly one blank
/// entry appears, and it always sorts last.
pub const BLANK_LAYOUT_NAME: &str = "blank-tab";

/// A tab of one plain shell, offered by the new-tab menu whether or not the config
/// declares any layout at all. It runs no harness, so it asks for nothing but a name.
pub fn blank_tab_layout() -> TabLayout {
    TabLayout {
        name: BLANK_LAYOUT_NAME.to_owned(),
        // U+2026, one character - not three full stops. The ellipsis marks the row as
        // opening a prompt, matching the usage menu's "let me write…".
        label: Some("Blank Tab\u{2026}".to_owned()),
        icon: Some("\u{f04e9}".to_owned()), // nf-md-tab
        tab_label: None,
        panes: vec![LayoutPane {
            name: "term".to_owned(),
            label: None,
            icon: None,
            pane_type: PaneType::Shell,
            agent: None,
            option_name: None,
            command: None,
            direction: None,
            ratio: None,
            split_from: None,
            env: BTreeMap::new(),
        }],
    }
}

fn default_tab_layouts() -> Vec<TabLayout> {
    vec![TabLayout {
        name: "agentic-coding".to_owned(),
        label: None,
        icon: None,
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

/// Icon and label joined by exactly two spaces, matching every existing menu label.
/// A missing icon renders the label alone, with no leading whitespace.
pub fn render_label(icon: Option<&str>, label: &str) -> String {
    match icon {
        Some(icon) => format!("{icon}  {label}"),
        None => label.to_owned(),
    }
}

/// The menu row for one named entry, falling back to its name when `label` is absent.
///
/// Every menu returns the selected row and maps it back to an entry by comparing it with
/// this exact string, so every site that renders an entry must go through here.
fn menu_label(icon: Option<&str>, label: Option<&str>, name: &str) -> String {
    render_label(icon, label.unwrap_or(name))
}

/// Validate the menu rows one config section renders.
///
/// `kind` names one entry ("layout"). The menu trims the centering pad off the selection
/// and `gum` trims its trailing whitespace, so a row with outer whitespace never survives
/// the round trip and is rejected here rather than mismatching — or resolving to the wrong
/// entry — at selection time. Two entries on the same row would both be listed while only
/// the first could ever be selected.
fn check_menu_rows<'a>(
    kind: &str,
    entries: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>,
) -> Result<()> {
    let mut rows: BTreeMap<String, &str> = BTreeMap::new();
    for (name, label, icon) in entries {
        if label.is_some_and(str::is_empty) {
            bail!("{kind} '{name}': label is empty; omit the key to fall back to the name");
        }
        if icon.is_some_and(str::is_empty) {
            bail!("{kind} '{name}': icon is empty; omit the key to render the label alone");
        }
        let row = menu_label(icon, label, name);
        if row.trim() != row {
            bail!("{kind} '{name}': menu label has leading or trailing whitespace: {row:?}");
        }
        match rows.entry(row) {
            Entry::Occupied(taken) => {
                bail!(
                    "{kind}s '{}' and '{name}' render the same menu label: {}",
                    taken.get(),
                    taken.key()
                );
            }
            Entry::Vacant(slot) => slot.insert(name),
        };
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self> {
        let home = required_env("HOME")?;
        let path = config_path(&home);
        // The zsh implementation exported `Q_WORKBENCH_LOCAL_CONFIG` pointing at
        // config.zsh, and that export outlives the cutover in any shell already running.
        // Parsing a zsh file as TOML fails on its first line, which reads as a corrupt
        // config rather than an unfinished migration — so name the real problem.
        if path.extension().is_some_and(|extension| extension == "zsh") {
            bail!(
                "config file {} is zsh, not TOML. Unset Q_WORKBENCH_LOCAL_CONFIG, \
                 or point it at a config.toml",
                path.display()
            );
        }
        let file: FileConfig = match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents)
                .with_context(|| format!("failed to parse config file {}", path.display()))?,
            Err(error) if error.kind() == ErrorKind::NotFound => Default::default(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read config file {}", path.display()));
            }
        };

        let config = Self {
            dashboard_workspace: resolve_string(
                file.dashboard_workspace,
                "Q_DASHBOARD_WORKSPACE",
                "personal-assistant",
            ),
            default_tab_layout: file
                .default_tab_layout
                .unwrap_or_else(|| "agentic-coding".to_owned()),
            project_registry_file: expand_home(
                resolve_string(
                    file.project_registry_file,
                    "Q_PROJECT_REGISTRY_FILE",
                    "$HOME/.local/state/herdr-projects/registry.json",
                ),
                &home,
            ),
            projects_root: expand_home(
                resolve_string(file.projects_root, "Q_PROJECTS_ROOT", "$HOME/Projects"),
                &home,
            ),
            ssh_registry_file: expand_home(
                resolve_string(
                    file.ssh_registry_file,
                    "Q_SSH_REGISTRY_FILE",
                    "$HOME/.local/state/ssh-targets/registry.json",
                ),
                &home,
            ),
            ssh_config_file: expand_home(
                resolve_string(
                    file.ssh_config_file,
                    "Q_SSH_CONFIG_FILE",
                    "$HOME/.config/ssh/config",
                ),
                &home,
            ),
            ssh_history_file: expand_home(
                resolve_string(
                    file.ssh_history_file,
                    "Q_SSH_HISTORY_FILE",
                    "$HOME/.zsh_history",
                ),
                &home,
            ),
            project_markers: file.project_markers.unwrap_or_else(default_project_markers),
            // User-written sections replace the built-in defaults entirely.
            // This is deliberate: "I only want codex" must be expressible.
            // Do not merge by name.
            tab_layouts: file.tab_layouts.unwrap_or_else(default_tab_layouts),
            agents: file.agents.unwrap_or_else(default_agents),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for marker in &self.project_markers {
            // The sweep compares a marker against one directory entry's file name, so
            // anything holding a separator can only ever fail to match, silently.
            if marker.is_empty() {
                bail!("project_markers: entry is empty");
            }
            if marker.contains('/') {
                bail!("project_markers: '{marker}' is a path, not a file name");
            }
        }

        let mut layout_names = BTreeSet::new();
        for layout in &self.tab_layouts {
            if !layout_names.insert(layout.name.as_str()) {
                bail!("duplicate tab layout name: {}", layout.name);
            }
        }
        check_menu_rows(
            "layout",
            self.tab_layouts.iter().map(|layout| {
                (
                    layout.name.as_str(),
                    layout.label.as_deref(),
                    layout.icon.as_deref(),
                )
            }),
        )?;

        let mut agent_names = BTreeSet::new();
        for agent in &self.agents {
            if !agent_names.insert(agent.name.as_str()) {
                bail!("duplicate agent name: {}", agent.name);
            }
            let mut option_names = BTreeSet::new();
            for option in &agent.options {
                if !option_names.insert(option.name.as_str()) {
                    bail!(
                        "agent '{}': duplicate option name: {}",
                        agent.name,
                        option.name
                    );
                }
            }
        }
        check_menu_rows(
            "agent",
            self.agents.iter().map(|agent| {
                (
                    agent.name.as_str(),
                    agent.label.as_deref(),
                    agent.icon.as_deref(),
                )
            }),
        )?;

        if !layout_names.contains(self.default_tab_layout.as_str()) {
            bail!(
                "default_tab_layout names no tab layout: {}",
                self.default_tab_layout
            );
        }

        for layout in &self.tab_layouts {
            if layout.panes.is_empty() {
                bail!(
                    "layout '{}': declares no panes; the first pane is the tab root",
                    layout.name
                );
            }

            let mut pane_names = BTreeSet::new();
            for (index, pane) in layout.panes.iter().enumerate() {
                if pane_names.contains(pane.name.as_str()) {
                    bail!(
                        "layout '{}': duplicate pane name: {}",
                        layout.name,
                        pane.name
                    );
                }

                if index == 0 {
                    if pane.split_from.is_some() {
                        bail!(
                            "layout '{}': the root pane '{}' cannot set split_from",
                            layout.name,
                            pane.name
                        );
                    }
                    if pane.direction.is_some() {
                        bail!(
                            "layout '{}': the root pane '{}' cannot set direction",
                            layout.name,
                            pane.name
                        );
                    }
                    if pane.ratio.is_some() {
                        bail!(
                            "layout '{}': the root pane '{}' cannot set ratio",
                            layout.name,
                            pane.name
                        );
                    }
                } else {
                    if let Some(target) = &pane.split_from {
                        if !pane_names.contains(target.as_str()) {
                            bail!(
                                "layout '{}' pane '{}': split_from names no earlier pane: {}",
                                layout.name,
                                pane.name,
                                target
                            );
                        }
                    }
                    if pane.direction.is_none() {
                        bail!(
                            "layout '{}' pane '{}': direction is required for a pane that splits",
                            layout.name,
                            pane.name
                        );
                    }
                    if pane.ratio.is_none() {
                        bail!(
                            "layout '{}' pane '{}': ratio is required for a pane that splits",
                            layout.name,
                            pane.name
                        );
                    }
                }

                if let Some(ratio) = pane.ratio {
                    // Every comparison against NaN is false, so this catches NaN for free.
                    if !(ratio > 0.0 && ratio < 1.0) {
                        bail!(
                            "layout '{}' pane '{}': ratio must be between 0 and 1, exclusive: {}",
                            layout.name,
                            pane.name,
                            ratio
                        );
                    }
                }

                match pane.pane_type {
                    PaneType::Command => match pane.command.as_deref() {
                        None => bail!(
                            "layout '{}' pane '{}': type = \"command\" requires command",
                            layout.name,
                            pane.name
                        ),
                        Some(command) if command.trim().is_empty() => bail!(
                            "layout '{}' pane '{}': command is empty",
                            layout.name,
                            pane.name
                        ),
                        Some(_) => {}
                    },
                    PaneType::Agent | PaneType::Shell if pane.command.is_some() => bail!(
                        "layout '{}' pane '{}': command is only valid for type = \"command\"",
                        layout.name,
                        pane.name
                    ),
                    PaneType::Agent | PaneType::Shell => {}
                }

                pane_names.insert(pane.name.as_str());
            }

            for pane in &layout.panes {
                // Only the agent pane launches a harness, so a pin on any other pane would
                // be accepted here and then silently dropped by the launch flow.
                if pane.pane_type != PaneType::Agent {
                    if pane.agent.is_some() {
                        bail!(
                            "layout '{}' pane '{}': agent is only valid for type = \"agent\"",
                            layout.name,
                            pane.name
                        );
                    }
                    if pane.option_name.is_some() {
                        bail!(
                            "layout '{}' pane '{}': option is only valid for type = \"agent\"",
                            layout.name,
                            pane.name
                        );
                    }
                }

                match (&pane.agent, &pane.option_name) {
                    (Some(agent_name), option_name) => {
                        let Some(agent) = self.agent(agent_name) else {
                            bail!(
                                "layout '{}' pane '{}': agent names no agent entry: {}",
                                layout.name,
                                pane.name,
                                agent_name
                            );
                        };
                        if let Some(option_name) = option_name {
                            if agent.option(option_name).is_none() {
                                bail!(
                                    "layout '{}' pane '{}': agent '{}' has no option: {}",
                                    layout.name,
                                    pane.name,
                                    agent_name,
                                    option_name
                                );
                            }
                        }
                    }
                    (None, Some(option_name)) => bail!(
                        "layout '{}' pane '{}': option requires agent, because an option belongs to one agent: {}",
                        layout.name,
                        pane.name,
                        option_name
                    ),
                    (None, None) => {}
                }
            }
        }

        for agent in &self.agents {
            // An empty executable is as unlaunchable as an empty vector, and both fail
            // only at exec time — after the tab is already on screen. Later argv entries
            // may legitimately be empty strings, so only the executable is checked.
            if agent
                .command
                .first()
                .is_none_or(|executable| executable.trim().is_empty())
            {
                bail!("agent '{}': command is empty", agent.name);
            }
            for option in &agent.options {
                if option.command.as_ref().is_some_and(|command| {
                    command
                        .first()
                        .is_none_or(|executable| executable.trim().is_empty())
                }) {
                    bail!(
                        "agent '{}' option '{}': command override is empty",
                        agent.name,
                        option.name
                    );
                }
            }
        }

        Ok(())
    }

    pub fn layout(&self, name: &str) -> Option<&TabLayout> {
        self.tab_layouts.iter().find(|layout| layout.name == name)
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.name == name)
    }
}

impl Agent {
    /// The harness menu row for this agent. See [`menu_label`].
    pub fn menu_label(&self) -> String {
        menu_label(self.icon.as_deref(), self.label.as_deref(), &self.name)
    }

    pub fn option(&self, name: &str) -> Option<&AgentOption> {
        self.options.iter().find(|option| option.name == name)
    }
}

impl LayoutPane {
    /// How this pane names itself in a menu title. See [`menu_label`].
    ///
    /// Only a layout with more than one agent pane needs it: the harness and model menus
    /// then run once per pane and have to say which pane they are asking about.
    pub fn menu_label(&self) -> String {
        menu_label(self.icon.as_deref(), self.label.as_deref(), &self.name)
    }
}

impl TabLayout {
    /// The layout menu row for this layout. See [`menu_label`].
    pub fn menu_label(&self) -> String {
        menu_label(self.icon.as_deref(), self.label.as_deref(), &self.name)
    }

    /// The layout's agent panes, paired with their index in `panes`, in config order.
    ///
    /// A layout may declare none. Such a tab runs no harness, so it skips the harness,
    /// model, and usage menus, and nothing writes restart state for it.
    pub fn agent_panes(&self) -> impl Iterator<Item = (usize, &LayoutPane)> {
        self.panes
            .iter()
            .enumerate()
            .filter(|(_, pane)| pane.pane_type == PaneType::Agent)
    }

    /// The pane an agent flow acts on when the caller named one, else the first agent
    /// pane. `None` when the layout declares no agent pane, or names one that is not.
    pub fn agent_pane(&self, name: Option<&str>) -> Option<(usize, &LayoutPane)> {
        match name {
            Some(name) => self.agent_panes().find(|(_, pane)| pane.name == name),
            None => self.agent_panes().next(),
        }
    }
}

fn config_path(home: &str) -> PathBuf {
    if let Some(path) = non_empty_env("Q_WORKBENCH_LOCAL_CONFIG") {
        return PathBuf::from(path);
    }

    let config_home = non_empty_env("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config"));
    PathBuf::from(config_home).join("herdr/plugins/config/q.workbench/config.toml")
}

fn required_env(name: &str) -> Result<String> {
    non_empty_env(name).with_context(|| format!("{name} must be set and non-empty"))
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn resolve_string(file: Option<String>, environment: &str, default: &str) -> String {
    file.or_else(|| non_empty_env(environment))
        .unwrap_or_else(|| default.to_owned())
}

fn expand_home(value: String, home: &str) -> String {
    if value == "$HOME" {
        return home.to_owned();
    }
    if let Some(rest) = value.strip_prefix("$HOME/") {
        return format!("{home}/{rest}");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::MutexGuard;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestEnvironment {
        _guard: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
        directory: PathBuf,
    }

    impl TestEnvironment {
        fn new() -> Self {
            let guard = crate::state::env_lock();
            let directory = env::temp_dir().join(format!(
                "workbench-config-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock is before Unix epoch")
                    .as_nanos()
            ));
            fs::create_dir_all(&directory).expect("create temporary directory");

            let names = [
                "HOME",
                "XDG_CONFIG_HOME",
                "Q_WORKBENCH_LOCAL_CONFIG",
                "Q_DASHBOARD_WORKSPACE",
                "Q_PROJECT_REGISTRY_FILE",
                "Q_PROJECTS_ROOT",
                "Q_SSH_REGISTRY_FILE",
                "Q_SSH_CONFIG_FILE",
                "Q_SSH_HISTORY_FILE",
            ];
            let saved = names
                .into_iter()
                .map(|name| (name, env::var_os(name)))
                .collect();
            for name in names {
                env::remove_var(name);
            }
            env::set_var("HOME", directory.join("home"));
            env::set_var("Q_WORKBENCH_LOCAL_CONFIG", directory.join("config.toml"));

            Self {
                _guard: guard,
                saved,
                directory,
            }
        }

        fn write(&self, contents: &str) {
            fs::write(self.directory.join("config.toml"), contents).expect("write config");
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
            fs::remove_dir_all(&self.directory).expect("remove temporary directory");
        }
    }

    /// A shell started before the cutover still exports the zsh-era override. The
    /// binary must name the stale override instead of reporting a TOML syntax
    /// error on `typeset -gA`, which is what every action showed on the first run.
    #[test]
    fn a_zsh_override_names_the_real_problem_instead_of_failing_to_parse_toml() {
        let environment = TestEnvironment::new();
        let legacy = environment.directory.join("config.zsh");
        fs::write(&legacy, "typeset -gA LEGACY_CONFIG\n").expect("write legacy config");
        env::set_var("Q_WORKBENCH_LOCAL_CONFIG", &legacy);

        let error = format!("{:#}", Config::load().expect_err("reject a zsh config"));

        assert!(error.contains("is zsh, not TOML"), "{error}");
        assert!(error.contains("Q_WORKBENCH_LOCAL_CONFIG"), "{error}");
        assert!(error.contains("config.toml"), "{error}");
    }

    #[test]
    fn missing_file_resolves_every_documented_default() {
        let environment = TestEnvironment::new();
        let home = environment.directory.join("home").display().to_string();
        let config = Config::load().expect("load defaults");

        assert_eq!(config.dashboard_workspace, "personal-assistant");
        assert_eq!(config.default_tab_layout, "agentic-coding");
        assert_eq!(
            config.project_registry_file,
            format!("{home}/.local/state/herdr-projects/registry.json")
        );
        assert_eq!(config.projects_root, format!("{home}/Projects"));
        assert_eq!(
            config.project_markers,
            ["package.json", "Gemfile", "Cargo.toml", "CLAUDE.md"]
        );
        assert_eq!(
            config.ssh_registry_file,
            format!("{home}/.local/state/ssh-targets/registry.json")
        );
        assert_eq!(config.ssh_config_file, format!("{home}/.config/ssh/config"));
        assert_eq!(config.ssh_history_file, format!("{home}/.zsh_history"));
        assert_eq!(config.tab_layouts, default_tab_layouts());
        assert_eq!(config.agents, default_agents());
    }

    #[test]
    fn project_markers_replace_the_defaults_and_reject_a_path() {
        let environment = TestEnvironment::new();
        environment.write("project_markers = [\"go.mod\", \"flake.nix\"]\n");

        let config = Config::load().expect("load markers");
        assert_eq!(config.project_markers, ["go.mod", "flake.nix"]);

        // An empty list is a real answer: it turns the marker sweep off and leaves
        // `.git` as the only thing that makes a directory a project.
        environment.write("project_markers = []\n");
        assert!(Config::load()
            .expect("load empty markers")
            .project_markers
            .is_empty());

        environment.write("project_markers = [\"config/database.yml\"]\n");
        let error = format!("{:#}", Config::load().expect_err("reject a path marker"));
        assert!(error.contains("is a path, not a file name"), "{error}");
    }

    #[test]
    fn missing_file_yields_the_shipping_harnesses_in_menu_order() {
        let _environment = TestEnvironment::new();
        let config = Config::load().expect("load defaults");
        let labels = config
            .agents
            .iter()
            .map(Agent::menu_label)
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            [
                "\u{f15ce}  claude code",
                "\u{ee0d}  codex",
                "\u{f169f}  opencode",
            ]
        );
    }

    #[test]
    fn default_agents_have_correct_options() {
        let agents = default_agents();
        let claude = &agents[0];

        assert_eq!(
            claude
                .options
                .iter()
                .map(|option| option.name.as_str())
                .collect::<Vec<_>>(),
            ["Opus", "OpusPlan (Sonnet)", "CCR", "Fable 5"]
        );
        assert_eq!(claude.options[0].args, ["--model", "claude-opus-4-8"]);
        assert_eq!(
            claude.options[1].args,
            ["--model", "opusplan", "--effort", "medium"]
        );
        assert!(claude.options[2].args.is_empty());
        assert_eq!(
            claude.options[2].command.as_deref(),
            Some(["ccr".to_owned(), "code".to_owned()].as_slice())
        );
        assert_eq!(claude.options[3].args, ["--model", "claude-fable-5"]);
        assert!(claude.options[0].command.is_none());
        assert!(claude.options[1].command.is_none());
        assert!(claude.options[3].command.is_none());
        assert!(agents[1].options.is_empty());
        assert!(agents[2].options.is_empty());
    }

    #[test]
    fn default_agents_have_empty_extra_args() {
        assert!(default_agents()
            .iter()
            .all(|agent| agent.extra_args.is_empty()));
    }

    #[test]
    fn user_agents_section_replaces_defaults() {
        let environment = TestEnvironment::new();
        environment.write(
            r#"
[[agents]]
name = "custom"
command = ["custom-agent"]
"#,
        );

        let config = Config::load().expect("load custom agents");

        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "custom");
        assert!(config.agent("claude code").is_none());
    }

    #[test]
    fn user_tab_layouts_section_replaces_defaults() {
        let environment = TestEnvironment::new();
        environment.write(
            r#"
default_tab_layout = "custom"

[[tab_layouts]]
name = "custom"

  [[tab_layouts.panes]]
  name = "agent"
  type = "agent"
"#,
        );

        let config = Config::load().expect("load custom layouts");

        assert_eq!(config.tab_layouts.len(), 1);
        assert_eq!(config.tab_layouts[0].name, "custom");
        assert!(config.layout("agentic-coding").is_none());
    }

    #[test]
    fn default_layout_has_correct_structure() {
        let layouts = default_tab_layouts();
        let layout = &layouts[0];

        assert_eq!(layouts.len(), 1);
        assert_eq!(layout.name, "agentic-coding");
        assert!(layout.tab_label.is_none());
        assert_eq!(layout.panes.len(), 3);

        let agent = &layout.panes[0];
        assert_eq!(agent.name, "agent");
        assert!(agent.label.is_none());
        assert!(agent.icon.is_none());
        assert_eq!(agent.pane_type, PaneType::Agent);
        assert!(agent.direction.is_none());
        assert!(agent.ratio.is_none());
        assert!(agent.split_from.is_none());
        assert_eq!(agent.env.get("Q_NO_BANNER").map(String::as_str), Some("1"));

        let files = &layout.panes[1];
        assert_eq!(files.name, "files");
        assert_eq!(files.label.as_deref(), Some("Files"));
        assert_eq!(files.icon.as_deref(), Some("\u{f0968}"));
        assert_eq!(files.pane_type, PaneType::Command);
        assert_eq!(files.command.as_deref(), Some("yazi ."));
        assert_eq!(files.direction, Some(Direction::Right));
        assert_eq!(files.ratio, Some(0.62));
        assert!(files.split_from.is_none());
        assert_eq!(files.env.get("Q_NO_BANNER").map(String::as_str), Some("1"));

        let term = &layout.panes[2];
        assert_eq!(term.name, "term");
        assert_eq!(term.label.as_deref(), Some("term"));
        assert_eq!(term.icon.as_deref(), Some("\u{f489}"));
        assert_eq!(term.pane_type, PaneType::Shell);
        assert!(term.command.is_none());
        assert_eq!(term.direction, Some(Direction::Down));
        assert_eq!(term.ratio, Some(0.1));
        assert!(term.split_from.is_none());
        assert!(term.env.is_empty());
    }

    #[test]
    fn default_tab_layout_resolves() {
        let config = Config::test_default();

        assert_eq!(config.default_tab_layout, "agentic-coding");
        assert!(config.layout(&config.default_tab_layout).is_some());
    }

    #[test]
    fn environment_overrides_built_in_defaults() {
        let _environment = TestEnvironment::new();
        env::set_var("Q_DASHBOARD_WORKSPACE", "from-environment");

        let config = Config::load().expect("load environment");

        assert_eq!(config.dashboard_workspace, "from-environment");
    }

    #[test]
    fn file_overrides_environment_including_with_empty_values() {
        let environment = TestEnvironment::new();
        env::set_var("Q_DASHBOARD_WORKSPACE", "from-environment");
        environment.write(
            r#"
dashboard_workspace = "from-file"
"#,
        );

        let config = Config::load().expect("load file");

        assert_eq!(config.dashboard_workspace, "from-file");
    }

    #[test]
    fn local_override_redirects_to_a_missing_file_without_error() {
        let environment = TestEnvironment::new();
        env::set_var(
            "Q_WORKBENCH_LOCAL_CONFIG",
            environment.directory.join("missing.toml"),
        );

        assert_eq!(
            Config::load().expect("load missing override"),
            Config::load().expect("load missing override again")
        );
    }

    #[test]
    fn default_path_uses_home_dot_config_when_xdg_config_home_is_unset() {
        let environment = TestEnvironment::new();
        env::remove_var("Q_WORKBENCH_LOCAL_CONFIG");

        assert_eq!(
            config_path(&environment.directory.join("home").display().to_string()),
            environment
                .directory
                .join("home/.config/herdr/plugins/config/q.workbench/config.toml")
        );
    }

    #[test]
    fn the_example_config_parses_with_no_unknown_fields() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
        let contents = fs::read_to_string(&path).expect("read config.example.toml");
        let file: FileConfig = toml::from_str(&contents).expect("parse config.example.toml");

        let layouts = file.tab_layouts.expect("example defines tab_layouts");
        assert_eq!(layouts.len(), 4);
        assert_eq!(layouts[0].name, "agentic-coding");
        assert_eq!(layouts[0].panes.len(), 3);
        assert_eq!(layouts[0].panes[0].pane_type, PaneType::Agent);
        assert!(layouts[0].label.is_none());
        assert!(layouts[0].icon.is_none());
        assert!(layouts[0].tab_label.is_none());
        assert_eq!(layouts[1].label.as_deref(), Some("Personal Assistant"));
        assert!(layouts[1].icon.is_none());
        assert!(layouts[1].tab_label.is_none());
        // The two shapes this file documents beyond one agent pane at the root.
        assert_eq!(layouts[2].agent_panes().count(), 2);
        assert_eq!(layouts[3].agent_panes().count(), 0);

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

    #[test]
    fn render_label_with_icon_uses_two_spaces() {
        let result = render_label(Some("\u{f15ce}"), "claude code");
        assert_eq!(result, "\u{f15ce}  claude code");
    }

    #[test]
    fn render_label_without_icon_returns_label_only() {
        let result = render_label(None, "term");
        assert_eq!(result, "term");
    }

    #[test]
    fn config_layout_returns_none_for_unknown_name() {
        assert_eq!(Config::test_default().layout("nonexistent"), None);
    }

    #[test]
    fn config_agent_returns_none_for_unknown_name() {
        assert_eq!(Config::test_default().agent("nonexistent"), None);
    }

    #[test]
    fn agent_option_returns_none_for_unknown_name() {
        let config = Config::test_default();
        let agent = config.agent("claude code").expect("default agent");
        assert_eq!(agent.option("nonexistent"), None);
        assert_eq!(
            agent.option("Opus").map(|option| option.name.as_str()),
            Some("Opus")
        );
    }

    fn validation_error(config: &Config) -> String {
        config
            .validate()
            .expect_err("config must be rejected")
            .to_string()
    }

    #[test]
    fn layout_menu_label_falls_back_to_name() {
        assert_eq!(
            Config::test_default().tab_layouts[0].menu_label(),
            "agentic-coding"
        );
    }

    #[test]
    fn layout_menu_label_uses_label_and_icon() {
        let mut layout = Config::test_default().tab_layouts.remove(0);
        layout.label = Some("Agentic Coding".to_owned());
        layout.icon = Some("A".to_owned());
        assert_eq!(layout.menu_label(), "A  Agentic Coding");
    }

    #[test]
    fn layout_menu_label_uses_label_without_icon() {
        let mut layout = Config::test_default().tab_layouts.remove(0);
        layout.label = Some("Agentic Coding".to_owned());
        assert_eq!(layout.menu_label(), "Agentic Coding");
    }

    fn load_layout_error(layouts: &str) -> String {
        let environment = TestEnvironment::new();
        environment.write(&format!("default_tab_layout = \"a\"\n{layouts}"));
        Config::load()
            .expect_err("reject invalid layout labels")
            .to_string()
    }

    #[test]
    fn layout_label_and_icon_parse_from_toml() {
        let environment = TestEnvironment::new();
        environment.write(
            r#"default_tab_layout = "a"
[[tab_layouts]]
name = "a"
label = "Work"
icon = "W"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#,
        );
        let config = Config::load().expect("load layout label and icon");
        let layout = config.layout("a").expect("parsed layout");
        assert_eq!(layout.label.as_deref(), Some("Work"));
        assert_eq!(layout.icon.as_deref(), Some("W"));
    }

    #[test]
    fn empty_layout_label_is_a_named_load_error() {
        let error = load_layout_error(
            r#"[[tab_layouts]]
name = "a"
label = ""
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#,
        );
        assert!(error.contains("layout 'a': label is empty"), "{error}");
    }

    #[test]
    fn empty_layout_icon_is_a_named_load_error() {
        let error = load_layout_error(
            r#"[[tab_layouts]]
name = "a"
icon = ""
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#,
        );
        assert!(error.contains("layout 'a': icon is empty"), "{error}");
    }

    // A padded row never survives the menu round trip: the flow trims the centering pad off
    // the selection, so " Work" would come back as "Work" and match another layout — or no
    // layout at all. Rejecting it at load keeps the reverse lookup total.
    #[test]
    fn layout_label_with_outer_whitespace_is_a_named_load_error() {
        for (key, value) in [("label", " Work"), ("label", "Work "), ("icon", " W")] {
            let error = load_layout_error(&format!(
                r#"[[tab_layouts]]
name = "a"
{key} = "{value}"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#
            ));
            assert!(
                error.contains("layout 'a': menu label has leading or trailing whitespace"),
                "{error}"
            );
        }
    }

    #[test]
    fn agent_label_with_outer_whitespace_is_a_named_load_error() {
        let mut config = Config::test_default();
        config.agents[0].icon = None;
        config.agents[0].label = Some(" claude".to_owned());

        let error = validation_error(&config);

        assert!(
            error.contains("menu label has leading or trailing whitespace"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_rendered_agent_labels_are_a_named_load_error() {
        let mut config = Config::test_default();
        let row = config.agents[0].menu_label();
        config.agents[1].icon = config.agents[0].icon.clone();
        config.agents[1].label = Some(
            config.agents[0]
                .label
                .clone()
                .unwrap_or_else(|| config.agents[0].name.clone()),
        );

        let error = validation_error(&config);

        assert!(
            error.contains(&format!("render the same menu label: {row}")),
            "{error}"
        );
    }

    #[test]
    fn duplicate_rendered_layout_labels_are_a_named_load_error() {
        let error = load_layout_error(
            r#"[[tab_layouts]]
name = "a"
label = "Work"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
[[tab_layouts]]
name = "b"
label = "Work"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#,
        );
        assert!(
            error.contains("layouts 'a' and 'b' render the same menu label: Work"),
            "{error}"
        );
    }

    #[test]
    fn layout_label_colliding_with_another_name_is_a_named_load_error() {
        let error = load_layout_error(
            r#"[[tab_layouts]]
name = "a"
label = "Work"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
[[tab_layouts]]
name = "Work"
[[tab_layouts.panes]]
name = "agent"
type = "agent"
"#,
        );
        assert!(
            error.contains("layouts 'a' and 'Work' render the same menu label: Work"),
            "{error}"
        );
    }

    #[test]
    fn unknown_default_tab_layout_is_a_named_error() {
        let mut config = Config::test_default();
        config.default_tab_layout = "missing-layout".to_owned();

        let error = validation_error(&config);

        assert!(error.contains("missing-layout"), "{error}");
    }

    #[test]
    fn unknown_pane_agent_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].name = "named-layout".to_owned();
        config.tab_layouts[0].panes[0].name = "named-pane".to_owned();
        config.tab_layouts[0].panes[0].agent = Some("missing-agent".to_owned());
        config.default_tab_layout = "named-layout".to_owned();

        let error = validation_error(&config);

        assert!(error.contains("named-layout"), "{error}");
        assert!(error.contains("named-pane"), "{error}");
        assert!(error.contains("missing-agent"), "{error}");
    }

    #[test]
    fn unknown_agent_option_is_a_named_error() {
        let mut config = Config::test_default();
        let pane = &mut config.tab_layouts[0].panes[0];
        pane.agent = Some("claude code".to_owned());
        pane.option_name = Some("missing-option".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
        assert!(error.contains("claude code"), "{error}");
        assert!(error.contains("missing-option"), "{error}");
    }

    #[test]
    fn option_without_agent_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].option_name = Some("orphan-option".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
        assert!(error.contains("orphan-option"), "{error}");
    }

    #[test]
    fn layout_without_panes_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes.clear();

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
    }

    #[test]
    fn a_layout_of_plain_shells_validates() {
        let mut config = Config::test_default();
        let layout = &mut config.tab_layouts[0];
        layout.tab_label = Some("Blank".to_owned());
        layout.panes.truncate(1);
        layout.panes[0].pane_type = PaneType::Shell;

        config.validate().expect("shell-only layout is valid");
        assert_eq!(config.tab_layouts[0].agent_panes().count(), 0);
    }

    #[test]
    fn several_agent_panes_validate() {
        let mut config = Config::test_default();
        // `files` is a command pane in the default layout, so its command has to go with
        // the type: command is rejected on an agent pane.
        let files = &mut config.tab_layouts[0].panes[1];
        files.pane_type = PaneType::Agent;
        files.command = None;

        config.validate().expect("two agent panes are valid");
        let agents = config.tab_layouts[0]
            .agent_panes()
            .map(|(index, pane)| (index, pane.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(agents, [(0, "agent"), (1, "files")]);
    }

    #[test]
    fn a_non_agent_root_keeps_its_agent_panes_in_config_order() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes.swap(0, 1);
        // The root takes no geometry, and the pane that moved off the root needs some.
        let root = &mut config.tab_layouts[0].panes[0];
        root.direction = None;
        root.ratio = None;
        let agent = &mut config.tab_layouts[0].panes[1];
        agent.direction = Some(Direction::Right);
        agent.ratio = Some(0.5);

        config.validate().expect("a command root is valid");
        assert_eq!(
            config.tab_layouts[0]
                .agent_pane(None)
                .map(|(index, _)| index),
            Some(1)
        );
    }

    #[test]
    fn agent_pane_selects_by_name_and_rejects_a_non_agent_pane() {
        let config = Config::test_default();
        let layout = &config.tab_layouts[0];

        assert_eq!(
            layout.agent_pane(Some("agent")).map(|(index, _)| index),
            Some(0)
        );
        assert!(layout.agent_pane(Some("term")).is_none());
        assert!(layout.agent_pane(Some("absent")).is_none());
    }

    #[test]
    fn duplicate_pane_name_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].name = "agent".to_owned();

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn unknown_or_later_split_target_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].split_from = Some("later-pane".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
        assert!(error.contains("later-pane"), "{error}");
    }

    #[test]
    fn root_split_from_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].split_from = Some("agent".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn missing_split_direction_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].direction = None;

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
    }

    #[test]
    fn root_direction_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].direction = Some(Direction::Right);

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn missing_split_ratio_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].ratio = None;

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
    }

    #[test]
    fn root_ratio_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].ratio = Some(0.5);

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn zero_ratio_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].ratio = Some(0.0);

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
        assert!(error.contains('0'), "{error}");
    }

    #[test]
    fn one_ratio_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].ratio = Some(1.0);

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
        assert!(error.contains('1'), "{error}");
    }

    #[test]
    fn nan_ratio_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].ratio = Some(f64::NAN);

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
    }

    #[test]
    fn command_pane_without_command_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].command = None;

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
    }

    #[test]
    fn command_on_non_command_pane_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].command = Some("echo nope".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn whitespace_pane_command_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].command = Some("  \t".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
    }

    #[test]
    fn empty_agent_command_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents[0].command.clear();

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
    }

    #[test]
    fn empty_option_command_override_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents[0].options[0].command = Some(Vec::new());

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
        assert!(error.contains("Opus"), "{error}");
    }

    #[test]
    fn whitespace_agent_command_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents[0].command = vec![" ".to_owned(), "--model".to_owned()];

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
    }

    #[test]
    fn whitespace_option_command_override_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents[0].options[0].command = Some(vec![String::new(), "code".to_owned()]);

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
        assert!(error.contains("Opus"), "{error}");
    }

    #[test]
    fn agent_on_a_non_agent_pane_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].agent = Some("codex".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn option_on_a_non_agent_pane_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].option_name = Some("Opus".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("files"), "{error}");
        assert!(error.contains("option"), "{error}");
    }

    #[test]
    fn duplicate_rendered_agent_label_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents[1].icon = config.agents[0].icon.clone();
        config.agents[1].label = Some("claude code".to_owned());

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
        assert!(error.contains("codex"), "{error}");
    }

    #[test]
    fn duplicate_layout_name_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts.push(config.tab_layouts[0].clone());

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
    }

    #[test]
    fn duplicate_agent_name_is_a_named_error() {
        let mut config = Config::test_default();
        config.agents.push(config.agents[0].clone());

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
    }

    #[test]
    fn duplicate_option_name_is_a_named_error() {
        let mut config = Config::test_default();
        let duplicate = config.agents[0].options[0].clone();
        config.agents[0].options.push(duplicate);

        let error = validation_error(&config);

        assert!(error.contains("claude code"), "{error}");
        assert!(error.contains("Opus"), "{error}");
    }

    #[test]
    fn no_config_file_returns_ok() {
        let _environment = TestEnvironment::new();

        assert!(Config::load().is_ok());
    }

    #[test]
    fn example_config_passes_validation() {
        let environment = TestEnvironment::new();
        environment.write(include_str!("../config.example.toml"));

        Config::load().expect("load and validate config.example.toml");
    }

    #[test]
    fn parse_stage_error_names_path_and_value() {
        let environment = TestEnvironment::new();
        environment.write(
            r#"
[[tab_layouts]]
name = "bad-direction"

  [[tab_layouts.panes]]
  name = "agent"
  type = "agent"

  [[tab_layouts.panes]]
  name = "term"
  type = "shell"
  direction = "left"
  ratio = 0.5
"#,
        );

        let error = format!(
            "{:#}",
            Config::load().expect_err("reject invalid direction")
        );
        let path = environment.directory.join("config.toml");

        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains("left"), "{error}");
    }
}
