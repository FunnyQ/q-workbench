use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What a `config.zsh` actually assigns, before any default applies.
///
/// Every field is an `Option` on purpose. `Config` has already resolved its defaults,
/// so it cannot tell "the user set this to the default" from "the user never mentioned
/// it" — and the migration must omit the second case, or an emitted default would pin a
/// value that should keep tracking future releases.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct PartialConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_extra_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_extra_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_registry_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_registry_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_config_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_history_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_args: Option<BTreeMap<String, Vec<String>>>,
}

const SCALAR_SETTINGS: &[(&str, &str)] = &[
    ("Q_DASHBOARD_WORKSPACE", "dashboard_workspace"),
    ("Q_CLAUDE_EXTRA_ARGS", "claude_extra_args"),
    ("Q_CODEX_EXTRA_ARGS", "codex_extra_args"),
    ("Q_PROJECT_REGISTRY_FILE", "project_registry_file"),
    ("Q_PROJECTS_ROOT", "projects_root"),
    ("Q_SSH_REGISTRY_FILE", "ssh_registry_file"),
    ("Q_SSH_CONFIG_FILE", "ssh_config_file"),
    ("Q_SSH_HISTORY_FILE", "ssh_history_file"),
];

/// Exit status the dump script uses when `source` itself failed. Distinct from any
/// status the sourced file could return, so a broken file is never mistaken for a
/// clean run that happened to set nothing.
const SOURCE_FAILED_STATUS: i32 = 3;

/// Convert an existing `config.zsh` into the settings it actually assigns.
///
/// There is no zsh parser here. zsh sources the file and reports the values, because
/// the file is a program: it may interpolate `$HOME`, branch on the host, or build the
/// model maps from a loop. Sourcing it is also exactly what the zsh plugin did on every
/// invocation, so it introduces no new risk — but it does execute the file, which the
/// subcommand's help text says out loud.
pub fn migrate(source: Option<&Path>) -> Result<PartialConfig> {
    let source = migration_source_path(source)?;
    if !source.is_file() {
        bail!("config source does not exist: {}", source.display());
    }

    let scalar_names = SCALAR_SETTINGS
        .iter()
        .map(|(shell, _)| *shell)
        .collect::<Vec<_>>()
        .join(" ");
    // `unset` before `source` is what makes "the variable is set" mean "the file set
    // it". Several of these settings are designed to be exported from Q's shell, and an
    // inherited value would otherwise be serialised as if the file had asked for it.
    // The unset list is built from the same name lists the dump iterates, so the two
    // can never drift apart.
    //
    // Records are NUL-delimited because model labels contain spaces and parentheses
    // (`OpusPlan (Sonnet)`); any whitespace-delimited format would split them. A record
    // is emitted only for a variable that is actually set — an absent setting must stay
    // absent in the output, not appear as an empty value.
    let script = format!(
        r#"
scalar_names=({scalar_names})
array_names=(Q_AGENT_MODEL_ORDER)
map_names=(Q_AGENT_MODELS Q_AGENT_MODEL_ARGS)
all_names=($scalar_names $array_names $map_names)
unset "${{all_names[@]}}"
source "$1"
source_status=$?
if (( source_status != 0 )); then
  print -u2 "sourcing the file exited with status $source_status"
  exit {SOURCE_FAILED_STATUS}
fi
for name in $scalar_names; do
  if (( ${{(P)+name}} )); then
    printf 'S\0%s\0%s\0' "$name" "${{(P)name}}"
  fi
done
for name in $array_names; do
  if (( ${{(P)+name}} )); then
    printf 'A\0%s\0' "$name"
    for value in "${{(P@)name}}"; do
      printf '%s\0' "$value"
    done
    printf '\0'
  fi
done
for name in $map_names; do
  if (( ${{(P)+name}} )); then
    printf 'M\0%s\0' "$name"
    for key in "${{(@kP)name}}"; do
      printf '%s\0%s\0' "$key" "${{${{(P)name}}[$key]}}"
    done
    printf '\0'
  fi
done
"#
    );
    let output = Command::new("zsh")
        .args(["-c", &script, "workbench-config-migrate"])
        .arg(&source)
        .output()
        .with_context(|| format!("failed to execute zsh for {}", source.display()))?;
    // A parse error inside the sourced file does not abort the `-c` wrapper: zsh prints
    // the error, `source` returns non-zero, and the wrapper carries on to report zero
    // settings with a successful exit. Without this check the migration would silently
    // emit an empty config and look like it worked.
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!("zsh failed while executing {}: {stderr}", source.display());
    }

    parse_dump(&output.stdout)
}

pub fn serialize_migration(config: &PartialConfig) -> Result<String> {
    let mut table = toml::Table::try_from(config).context("failed to serialize migrated config")?;
    // The two model tables are rendered separately, so that every key can be quoted
    // regardless of what it contains. Pulling them out first also keeps them at the end
    // of the file, where TOML requires tables to sit.
    let models = table.remove("models");
    let model_args = table.remove("model_args");

    let mut output = String::new();
    if config.claude_extra_args.is_some() || config.codex_extra_args.is_some() {
        output.push_str(
            "# Extra-args arrays can now contain arguments with spaces; config.zsh could not.\n",
        );
    }
    output
        .push_str(&toml::to_string_pretty(&table).context("failed to serialize migrated config")?);
    push_model_table(&mut output, "models", models.as_ref())?;
    push_model_table(&mut output, "model_args", model_args.as_ref())?;
    Ok(output)
}

/// Render one model table with every key quoted.
///
/// `OpusPlan (Sonnet)` is not a bare TOML key, and quoting only the keys that need it
/// would make the file's shape depend on which labels Q happens to use. Values go
/// through `toml::Value`'s own inline rendering rather than any string formatting here.
fn push_model_table(output: &mut String, name: &str, table: Option<&toml::Value>) -> Result<()> {
    let Some(table) = table else {
        return Ok(());
    };
    let table = table
        .as_table()
        .with_context(|| format!("{name} did not serialize to a TOML table"))?;

    output.push_str(&format!("\n[{name}]\n"));
    for (key, value) in table {
        let key = toml::Value::String(key.clone()).to_string();
        output.push_str(&format!("{key} = {value}\n"));
    }
    Ok(())
}

pub fn resolved_config_path() -> Result<PathBuf> {
    Ok(config_path(&required_env("HOME")?))
}

/// The `config.zsh` a migration reads: `--from` when given, else the legacy path.
pub fn migration_source_path(source: Option<&Path>) -> Result<PathBuf> {
    match source {
        Some(path) => Ok(path.to_path_buf()),
        None => Ok(legacy_config_path(&required_env("HOME")?)),
    }
}

fn legacy_config_path(home: &str) -> PathBuf {
    let config_home = non_empty_env("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config"));
    PathBuf::from(config_home).join("herdr/plugins/config/q.workbench/config.zsh")
}

/// Decode the dump script's output.
///
/// The wire format is a flat NUL-delimited stream. A scalar is `S`, name, value. An
/// array is `A`, name, each element, then an empty field. A map is `M`, name, then
/// alternating key and value, then an empty field. The trailing empty field is what
/// terminates a variable-length record, so an empty field can never appear inside one.
fn parse_dump(bytes: &[u8]) -> Result<PartialConfig> {
    let fields = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut index = 0;
    let mut config = PartialConfig::default();
    while index < fields.len() && !fields[index].is_empty() {
        let kind = dump_string(fields[index])?;
        index += 1;
        let name = take_dump_string(&fields, &mut index)?;
        match kind.as_str() {
            "S" => {
                let value = take_dump_string(&fields, &mut index)?;
                set_scalar(&mut config, &name, value)?;
            }
            "A" => {
                let values = take_record_values(&fields, &mut index)?;
                if name == "Q_AGENT_MODEL_ORDER" {
                    config.order = Some(values);
                } else {
                    bail!("unknown array setting from zsh: {name}");
                }
            }
            "M" => {
                let values = take_record_values(&fields, &mut index)?;
                if values.len() % 2 != 0 {
                    bail!("invalid map record from zsh: {name}");
                }
                let pairs = values
                    .chunks_exact(2)
                    .map(|pair| (pair[0].clone(), pair[1].clone()));
                match name.as_str() {
                    "Q_AGENT_MODELS" => config.models = Some(pairs.collect()),
                    "Q_AGENT_MODEL_ARGS" => {
                        config.model_args = Some(
                            pairs
                                .map(|(key, value)| {
                                    let args =
                                        value.split_whitespace().map(str::to_owned).collect();
                                    (key, args)
                                })
                                .collect(),
                        );
                    }
                    _ => bail!("unknown map setting from zsh: {name}"),
                }
            }
            _ => bail!("unknown record type from zsh: {kind}"),
        }
    }
    Ok(config)
}

fn take_dump_string(fields: &[&[u8]], index: &mut usize) -> Result<String> {
    let field = fields
        .get(*index)
        .with_context(|| "truncated output from zsh")?;
    *index += 1;
    dump_string(field)
}

fn take_record_values(fields: &[&[u8]], index: &mut usize) -> Result<Vec<String>> {
    let mut values = Vec::new();
    while let Some(field) = fields.get(*index) {
        *index += 1;
        if field.is_empty() {
            return Ok(values);
        }
        values.push(dump_string(field)?);
    }
    bail!("unterminated output record from zsh")
}

fn dump_string(value: &[u8]) -> Result<String> {
    String::from_utf8(value.to_vec()).context("zsh emitted a non-UTF-8 setting")
}

/// Assign one dumped scalar.
///
/// The two extra-args settings were `${=Q_…_EXTRA_ARGS}` word-split strings, so
/// splitting on whitespace here reproduces exactly what zsh did with them.
fn set_scalar(config: &mut PartialConfig, shell_name: &str, value: String) -> Result<()> {
    match shell_name {
        "Q_DASHBOARD_WORKSPACE" => config.dashboard_workspace = Some(value),
        "Q_CLAUDE_EXTRA_ARGS" => {
            config.claude_extra_args = Some(value.split_whitespace().map(str::to_owned).collect())
        }
        "Q_CODEX_EXTRA_ARGS" => {
            config.codex_extra_args = Some(value.split_whitespace().map(str::to_owned).collect())
        }
        "Q_PROJECT_REGISTRY_FILE" => config.project_registry_file = Some(value),
        "Q_PROJECTS_ROOT" => config.projects_root = Some(value),
        "Q_SSH_REGISTRY_FILE" => config.ssh_registry_file = Some(value),
        "Q_SSH_CONFIG_FILE" => config.ssh_config_file = Some(value),
        "Q_SSH_HISTORY_FILE" => config.ssh_history_file = Some(value),
        _ => bail!("unknown scalar setting from zsh: {shell_name}"),
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self> {
        let home = required_env("HOME")?;
        let path = config_path(&home);
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

    #[test]
    fn fixture_migrates_all_set_values_and_omits_absent_values() {
        let environment = TestEnvironment::new();
        env::remove_var("Q_WORKBENCH_LOCAL_CONFIG");
        env::set_var("Q_SSH_HISTORY_FILE", "/inherited/not-in-source");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.fixture");

        let partial = migrate(Some(&fixture)).expect("migrate fixture");
        let toml = serialize_migration(&partial).expect("serialize migration");

        assert_eq!(
            partial.dashboard_workspace.as_deref(),
            Some("personal-assistant")
        );
        assert_eq!(
            partial.claude_extra_args,
            Some(["--permission-mode", "plan"].map(str::to_owned).to_vec())
        );
        assert_eq!(
            partial.codex_extra_args,
            Some(
                ["--search", "--profile", "work"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
        assert_eq!(partial.order, Some(default_order()));
        assert_eq!(
            partial
                .models
                .as_ref()
                .and_then(|models| models.get("OpusPlan (Sonnet)"))
                .map(String::as_str),
            Some("opusplan")
        );
        assert_eq!(
            partial
                .model_args
                .as_ref()
                .and_then(|args| args.get("OpusPlan (Sonnet)")),
            Some(&["--effort", "medium"].map(str::to_owned).to_vec())
        );
        assert!(partial.ssh_history_file.is_none());
        assert!(toml.contains("dashboard_workspace = \"personal-assistant\""));
        assert!(!toml.contains("ssh_history_file"));
        assert!(toml.contains("\"Opus\" = \"claude-opus-4-8\""));
        assert!(toml.contains("\"CCR\" = \"CCR\""));
        assert!(toml.contains("\"OpusPlan (Sonnet)\" = \"opusplan\""));

        env::remove_var("Q_SSH_HISTORY_FILE");
        environment.write(&toml);
        let resolved = Config::load().expect("load migrated TOML");
        assert_eq!(resolved.dashboard_workspace, "personal-assistant");
        assert_eq!(
            resolved.ssh_history_file,
            environment
                .directory
                .join("home/.zsh_history")
                .display()
                .to_string()
        );
        assert_eq!(resolved.order, default_order());
        assert_eq!(
            resolved.models.get("Fable 5").map(String::as_str),
            Some("claude-fable-5")
        );
    }

    #[test]
    fn missing_migration_source_is_a_clear_error() {
        let environment = TestEnvironment::new();
        let missing = environment.directory.join("missing.zsh");

        let error = migrate(Some(&missing)).expect_err("reject missing source");

        assert!(error.to_string().contains("does not exist"));
        assert!(error.to_string().contains("missing.zsh"));
    }

    #[test]
    fn a_source_zsh_cannot_parse_is_an_error_not_an_empty_config() {
        let environment = TestEnvironment::new();
        let source = environment.directory.join("broken.zsh");
        fs::write(&source, "Q_PROJECTS_ROOT=(\n").expect("write broken source");

        let error = migrate(Some(&source)).expect_err("reject an unparseable source");

        assert!(error.to_string().contains("parse error"), "{error:#}");
        assert!(error.to_string().contains("broken.zsh"), "{error:#}");
    }

    #[test]
    fn model_labels_that_look_like_assignments_survive_serialization() {
        let environment = TestEnvironment::new();
        let source = environment.directory.join("odd-labels.zsh");
        fs::write(
            &source,
            concat!(
                "typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS\n",
                "Q_AGENT_MODEL_ORDER=('a = b')\n",
                "Q_AGENT_MODELS=('a = b' 'model-one')\n",
                "Q_AGENT_MODEL_ARGS=('a = b' '--effort medium')\n",
            ),
        )
        .expect("write source");

        let partial = migrate(Some(&source)).expect("migrate odd labels");
        let toml = serialize_migration(&partial).expect("serialize migration");
        let reparsed: FileConfig = toml::from_str(&toml).expect("reparse emitted TOML");

        assert_eq!(
            reparsed
                .models
                .as_ref()
                .and_then(|models| models.get("a = b"))
                .map(String::as_str),
            Some("model-one")
        );
        assert_eq!(
            reparsed
                .model_args
                .as_ref()
                .and_then(|args| args.get("a = b")),
            Some(&["--effort", "medium"].map(str::to_owned).to_vec())
        );
    }

    #[test]
    fn the_extra_args_note_appears_only_when_an_extra_args_setting_is_migrated() {
        let environment = TestEnvironment::new();
        let source = environment.directory.join("no-extra-args.zsh");
        fs::write(&source, "Q_DASHBOARD_WORKSPACE='work'\n").expect("write source");

        let partial = migrate(Some(&source)).expect("migrate source");
        let toml = serialize_migration(&partial).expect("serialize migration");

        assert!(!toml.contains("arguments with spaces"), "{toml}");
    }

    /// Read-only smoke test against Q's own file, skipped when it is absent. It never
    /// writes anywhere — the migration's only job here is to prove the real file still
    /// round-trips through the loader.
    #[test]
    fn the_real_config_zsh_round_trips_when_it_exists() {
        let environment = TestEnvironment::new();
        let Some(home) = environment
            .saved
            .iter()
            .find(|(name, _)| *name == "HOME")
            .and_then(|(_, value)| value.clone())
        else {
            return;
        };
        let source = legacy_config_path(&home.to_string_lossy());
        if !source.is_file() {
            return;
        }

        let partial = migrate(Some(&source)).expect("migrate the real config.zsh");
        let toml = serialize_migration(&partial).expect("serialize the real config.zsh");
        environment.write(&toml);
        let resolved = Config::load().expect("load the migrated real config");

        let dumped = Command::new("zsh")
            .args([
                "-c",
                "source \"$1\"; print -r -- \"${Q_DASHBOARD_WORKSPACE:-personal-assistant}\"; \
                 print -rl -- \"${Q_AGENT_MODEL_ORDER[@]}\"",
                "workbench-smoke",
            ])
            .arg(&source)
            .output()
            .expect("source the real config.zsh");
        let mut lines = std::str::from_utf8(&dumped.stdout)
            .expect("UTF-8 zsh output")
            .lines();

        assert_eq!(lines.next(), Some(resolved.dashboard_workspace.as_str()));
        let order = lines.map(str::to_owned).collect::<Vec<_>>();
        if !order.is_empty() {
            assert_eq!(resolved.order, order);
        }
    }

    #[test]
    fn default_migration_source_uses_xdg_config_home() {
        let environment = TestEnvironment::new();
        env::remove_var("Q_WORKBENCH_LOCAL_CONFIG");
        let config_home = environment.directory.join("xdg");
        let source = config_home.join("herdr/plugins/config/q.workbench/config.zsh");
        fs::create_dir_all(source.parent().expect("source parent")).expect("create source parent");
        fs::write(&source, "Q_PROJECTS_ROOT='/from-default-path'\n").expect("write source");
        env::set_var("XDG_CONFIG_HOME", config_home);

        let partial = migrate(None).expect("migrate default source");

        assert_eq!(partial.projects_root.as_deref(), Some("/from-default-path"));
    }
}
