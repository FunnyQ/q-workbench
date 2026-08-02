use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
// Tests only: production code must reach a Config through `load`, which is the one
// place the documented defaults live.
#[cfg_attr(test, derive(Default))]
pub struct Config {
    pub dashboard_workspace: String,
    pub claude_extra_args: Vec<String>,
    pub codex_extra_args: Vec<String>,
    pub project_registry_file: String,
    pub projects_root: String,
    pub ssh_registry_file: String,
    pub ssh_config_file: String,
    pub ssh_history_file: String,
    pub order: Vec<String>,
    pub models: BTreeMap<String, String>,
    pub model_args: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    dashboard_workspace: Option<String>,
    claude_extra_args: Option<Vec<String>>,
    codex_extra_args: Option<Vec<String>>,
    project_registry_file: Option<String>,
    projects_root: Option<String>,
    ssh_registry_file: Option<String>,
    ssh_config_file: Option<String>,
    ssh_history_file: Option<String>,
    order: Option<Vec<String>>,
    models: Option<BTreeMap<String, String>>,
    model_args: Option<BTreeMap<String, Vec<String>>>,
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

        let order_from_file = file.order.is_some();
        let models_from_file = file.models.is_some();
        let model_args_from_file = file.model_args.is_some();
        let mut config = Self {
            dashboard_workspace: resolve_string(
                file.dashboard_workspace,
                "Q_DASHBOARD_WORKSPACE",
                "personal-assistant",
            ),
            claude_extra_args: resolve_args(file.claude_extra_args, "Q_CLAUDE_EXTRA_ARGS"),
            codex_extra_args: resolve_args(file.codex_extra_args, "Q_CODEX_EXTRA_ARGS"),
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
            order: file.order.unwrap_or_else(default_order),
            models: file.models.unwrap_or_else(default_models),
            model_args: file.model_args.unwrap_or_else(default_model_args),
        };

        apply_model_environment(
            &mut config,
            order_from_file,
            models_from_file,
            model_args_from_file,
        )?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for label in &self.order {
            if !self.models.contains_key(label) {
                bail!("model order label has no model entry: {label}");
            }
        }
        Ok(())
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

fn resolve_args(file: Option<Vec<String>>, environment: &str) -> Vec<String> {
    file.unwrap_or_else(|| {
        non_empty_env(environment)
            .map(|value| value.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default()
    })
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

fn default_order() -> Vec<String> {
    ["Opus", "OpusPlan (Sonnet)", "CCR", "Fable 5"]
        .map(str::to_owned)
        .to_vec()
}

fn default_models() -> BTreeMap<String, String> {
    [
        ("Opus", "claude-opus-4-8"),
        ("OpusPlan (Sonnet)", "opusplan"),
        ("CCR", "CCR"),
        ("Fable 5", "claude-fable-5"),
    ]
    .map(|(label, model)| (label.to_owned(), model.to_owned()))
    .into()
}

fn default_model_args() -> BTreeMap<String, Vec<String>> {
    [(
        "OpusPlan (Sonnet)".to_owned(),
        vec!["--effort".to_owned(), "medium".to_owned()],
    )]
    .into()
}

fn apply_model_environment(
    config: &mut Config,
    order_from_file: bool,
    models_from_file: bool,
    model_args_from_file: bool,
) -> Result<()> {
    if !order_from_file {
        if let Some(value) = non_empty_env("Q_AGENT_MODEL_ORDER") {
            config.order = parse_environment_toml("Q_AGENT_MODEL_ORDER", &value)?;
        }
    }
    if !models_from_file {
        if let Some(value) = non_empty_env("Q_AGENT_MODELS") {
            config.models = parse_environment_toml("Q_AGENT_MODELS", &value)?;
        }
    }
    if !model_args_from_file {
        if let Some(value) = non_empty_env("Q_AGENT_MODEL_ARGS") {
            config.model_args = parse_environment_toml("Q_AGENT_MODEL_ARGS", &value)?;
        }
    }
    Ok(())
}

fn parse_environment_toml<T>(name: &str, value: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper<T> {
        value: T,
    }

    toml::from_str::<Wrapper<T>>(&format!("value = {value}"))
        .map(|wrapper| wrapper.value)
        .with_context(|| format!("{name} must contain a TOML value"))
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
                "Q_CLAUDE_EXTRA_ARGS",
                "Q_CODEX_EXTRA_ARGS",
                "Q_PROJECT_REGISTRY_FILE",
                "Q_PROJECTS_ROOT",
                "Q_SSH_REGISTRY_FILE",
                "Q_SSH_CONFIG_FILE",
                "Q_SSH_HISTORY_FILE",
                "Q_AGENT_MODEL_ORDER",
                "Q_AGENT_MODELS",
                "Q_AGENT_MODEL_ARGS",
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
        fs::write(&legacy, "typeset -gA Q_AGENT_MODELS\n").expect("write legacy config");
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
        assert!(config.claude_extra_args.is_empty());
        assert!(config.codex_extra_args.is_empty());
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
        assert_eq!(config.order, default_order());
        assert_eq!(config.models, default_models());
        assert_eq!(config.model_args, default_model_args());
    }

    #[test]
    fn environment_overrides_built_in_defaults_and_splits_extra_args() {
        let _environment = TestEnvironment::new();
        env::set_var("Q_DASHBOARD_WORKSPACE", "from-environment");
        env::set_var("Q_CODEX_EXTRA_ARGS", "--search --profile work");

        let config = Config::load().expect("load environment");

        assert_eq!(config.dashboard_workspace, "from-environment");
        assert_eq!(
            config.codex_extra_args,
            ["--search", "--profile", "work"].map(str::to_owned)
        );
    }

    #[test]
    fn file_overrides_environment_including_with_empty_values() {
        let environment = TestEnvironment::new();
        env::set_var("Q_DASHBOARD_WORKSPACE", "from-environment");
        env::set_var("Q_CODEX_EXTRA_ARGS", "--from-environment");
        environment.write(
            r#"
dashboard_workspace = "from-file"
codex_extra_args = []
"#,
        );

        let config = Config::load().expect("load file");

        assert_eq!(config.dashboard_workspace, "from-file");
        assert!(config.codex_extra_args.is_empty());
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
    fn file_extra_args_preserve_spaces_inside_one_argument() {
        let environment = TestEnvironment::new();
        environment.write(r#"claude_extra_args = ["--prompt", "two words"]"#);

        let config = Config::load().expect("load file arguments");

        assert_eq!(
            config.claude_extra_args,
            ["--prompt", "two words"].map(str::to_owned)
        );
    }

    #[test]
    fn extra_args_never_gain_implicit_bypass_flags() {
        let _environment = TestEnvironment::new();

        let config = Config::load().expect("load defaults");

        assert!(config.claude_extra_args.is_empty());
        assert!(config.codex_extra_args.is_empty());
    }

    #[test]
    fn model_order_label_without_model_is_a_named_error() {
        let environment = TestEnvironment::new();
        environment.write(
            r#"
order = ["Missing"]
models = {}
"#,
        );

        let error = Config::load().expect_err("reject invalid model order");

        assert!(error.to_string().contains("Missing"));
    }
}
