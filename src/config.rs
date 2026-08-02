use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
// Tests only: production code must reach a Config through `load`, which is the one
// place the documented defaults live.
#[cfg_attr(test, derive(Default))]
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
        let file = match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents)
                .with_context(|| format!("failed to parse config file {}", path.display()))?,
            Err(error) if error.kind() == ErrorKind::NotFound => FileConfig::default(),
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
            tab_layouts: file.tab_layouts.unwrap_or_default(),
            agents: file.agents.unwrap_or_default(),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        Ok(())
    }

    pub fn layout(&self, name: &str) -> Option<&TabLayout> {
        self.tab_layouts.iter().find(|layout| layout.name == name)
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|agent| agent.name == name)
    }

    /// Bridge to the pre-schema call sites: the one agent that carries a model menu.
    ///
    /// Temporary. The launch flow resolves its agent from the layout instead, and this
    /// goes away with the last caller.
    pub(crate) fn menu_agent(&self) -> Option<&Agent> {
        self.agents.iter().find(|agent| !agent.options.is_empty())
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
    use std::sync::{Mutex, MutexGuard};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENVIRONMENT: Mutex<()> = Mutex::new(());

    struct TestEnvironment {
        _guard: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
        directory: PathBuf,
    }

    impl TestEnvironment {
        fn new() -> Self {
            let guard = ENVIRONMENT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        assert!(config.tab_layouts.is_empty());
        assert!(config.agents.is_empty());
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
        let config = Config {
            dashboard_workspace: "default".to_owned(),
            default_tab_layout: "agentic-coding".to_owned(),
            project_registry_file: "projects.json".to_owned(),
            projects_root: "/home/user/projects".to_owned(),
            ssh_registry_file: "ssh.json".to_owned(),
            ssh_config_file: "~/.ssh/config".to_owned(),
            ssh_history_file: "~/.ssh/history".to_owned(),
            tab_layouts: vec![],
            agents: vec![],
        };
        assert_eq!(config.layout("nonexistent"), None);
    }

    #[test]
    fn config_agent_returns_none_for_unknown_name() {
        let config = Config {
            dashboard_workspace: "default".to_owned(),
            default_tab_layout: "agentic-coding".to_owned(),
            project_registry_file: "projects.json".to_owned(),
            projects_root: "/home/user/projects".to_owned(),
            ssh_registry_file: "ssh.json".to_owned(),
            ssh_config_file: "~/.ssh/config".to_owned(),
            ssh_history_file: "~/.ssh/history".to_owned(),
            tab_layouts: vec![],
            agents: vec![],
        };
        assert_eq!(config.agent("nonexistent"), None);
    }
}
