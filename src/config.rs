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

/// Icon and label joined by exactly two spaces, matching every existing menu label.
/// A missing icon renders the label alone, with no leading whitespace.
pub fn render_label(icon: Option<&str>, label: &str) -> String {
    match icon {
        Some(icon) => format!("{icon}  {label}"),
        None => label.to_owned(),
    }
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
        let mut layout_names = BTreeSet::new();
        for layout in &self.tab_layouts {
            if !layout_names.insert(layout.name.as_str()) {
                bail!("duplicate tab layout name: {}", layout.name);
            }
        }

        let mut agent_names = BTreeSet::new();
        let mut agent_labels: BTreeMap<String, &str> = BTreeMap::new();
        for agent in &self.agents {
            if !agent_names.insert(agent.name.as_str()) {
                bail!("duplicate agent name: {}", agent.name);
            }

            // The harness menu returns the rendered row and maps it back to an agent by
            // that string. Two agents rendering the same row would both be listed while
            // only the first could ever be selected.
            let label = agent.menu_label();
            if let Some(other) = agent_labels.insert(label.clone(), agent.name.as_str()) {
                bail!(
                    "agents '{}' and '{}' render the same menu label: {}",
                    other,
                    agent.name,
                    label
                );
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

            let root = &layout.panes[0];
            if root.pane_type != PaneType::Agent {
                bail!(
                    "layout '{}': the first pane is the tab root and must be type = \"agent\", found {:?}",
                    layout.name,
                    root.pane_type
                );
            }

            let agent_pane_count = layout
                .panes
                .iter()
                .filter(|pane| pane.pane_type == PaneType::Agent)
                .count();
            if agent_pane_count != 1 {
                bail!(
                    "layout '{}': exactly one pane may be type = \"agent\", found {}",
                    layout.name,
                    agent_pane_count
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
    /// The harness menu row for this agent. The reverse lookup in the harness menu matches
    /// on this exact string, so every site that renders an agent must go through here.
    pub fn menu_label(&self) -> String {
        render_label(
            self.icon.as_deref(),
            self.label.as_deref().unwrap_or(&self.name),
        )
    }

    pub fn option(&self, name: &str) -> Option<&AgentOption> {
        self.options.iter().find(|option| option.name == name)
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
            config.ssh_registry_file,
            format!("{home}/.local/state/ssh-targets/registry.json")
        );
        assert_eq!(config.ssh_config_file, format!("{home}/.config/ssh/config"));
        assert_eq!(config.ssh_history_file, format!("{home}/.zsh_history"));
        assert_eq!(config.tab_layouts, default_tab_layouts());
        assert_eq!(config.agents, default_agents());
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
    fn non_agent_root_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[0].pane_type = PaneType::Shell;

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains("Shell"), "{error}");
    }

    #[test]
    fn multiple_agent_panes_is_a_named_error() {
        let mut config = Config::test_default();
        config.tab_layouts[0].panes[1].pane_type = PaneType::Agent;

        let error = validation_error(&config);

        assert!(error.contains("agentic-coding"), "{error}");
        assert!(error.contains('2'), "{error}");
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
