use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};

#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod flows;
#[allow(dead_code)]
mod herdr;
#[allow(dead_code)]
mod notify;
mod registry;
#[allow(dead_code)]
mod shell;

use herdr::{check_protocol, HerdrClient, ProtocolGuardError, SocketClient};

#[derive(Debug, Parser)]
#[command(name = "workbench", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Ssh {
        #[command(subcommand)]
        command: SshCommand,
    },
    Dashboard,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Herdr {
        #[command(subcommand)]
        command: HerdrCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    Popup {
        #[arg(long)]
        worktree: bool,
    },
    Launch(LaunchArgs),
    Inject(InjectArgs),
    Restart,
    #[command(hide = true)]
    RestartWorker {
        #[arg(long)]
        pane: String,
    },
}

#[derive(Debug, Args)]
struct LaunchArgs {
    pane_id: String,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    usage: Option<String>,
    #[arg(long)]
    worktree: bool,
    #[arg(long)]
    no_layout: bool,
    #[arg(long, hide = true)]
    restart: bool,
}

#[derive(Debug, Args)]
struct InjectArgs {
    pane_id: String,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    usage: Option<String>,
    #[arg(long)]
    worktree: bool,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Pick,
    Source { query: Option<String> },
    Scan,
    Rescan,
    Update,
    Use { path: Option<PathBuf> },
    Edit { path: PathBuf },
}

#[derive(Debug, Subcommand)]
enum SshCommand {
    Pick,
    Sync,
    List,
    Get { target: String },
    Use { target: String },
    Remove { target: String },
    Edit { target: Option<String> },
    Session { target: String, tab_id: String },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Migrate config.zsh to TOML.
    ///
    /// The source file is executed by zsh.
    ///
    /// Example: `workbench config migrate --from ./config.zsh`, review the
    /// output, then run `workbench config migrate --from ./config.zsh --write`.
    ///
    /// `--from` defaults to
    /// `${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench/config.zsh`.
    /// `--write` writes to the resolved config.toml path and refuses to
    /// overwrite it without `--force`. By default, the TOML is printed to
    /// stdout. `--write` always refuses a destination that is the source
    /// itself, or that is not named `.toml`.
    Migrate {
        /// The config.zsh to read. zsh executes it.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,
        /// Install the result at the resolved config.toml path.
        #[arg(long)]
        write: bool,
        /// Allow --write to replace an existing config.toml.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum HerdrCommand {
    Ping,
}

impl Cli {
    fn run(self) -> Result<()> {
        let client = if self.uses_herdr() {
            let client = SocketClient::new()?;
            self.guard_protocol(&client)?;
            Some(client)
        } else {
            None
        };

        let path = match self.command {
            Command::Agent { command } => match command {
                AgentCommand::Popup { .. } => "agent popup",
                AgentCommand::Launch(_) => "agent launch",
                AgentCommand::Inject(_) => "agent inject",
                AgentCommand::Restart => "agent restart",
                AgentCommand::RestartWorker { .. } => "agent restart-worker",
            },
            Command::Project { command } => match command {
                ProjectCommand::Pick => "project pick",
                ProjectCommand::Source { query } => {
                    flows::picker::project_source(query.as_deref())?;
                    return Ok(());
                }
                ProjectCommand::Scan => {
                    let config = config::Config::load()?;
                    registry::project::scan(Path::new(&config.project_registry_file))?;
                    return Ok(());
                }
                ProjectCommand::Rescan => {
                    let config = config::Config::load()?;
                    registry::project::rescan(Path::new(&config.project_registry_file))?;
                    return Ok(());
                }
                ProjectCommand::Update => {
                    let config = config::Config::load()?;
                    let home = std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .context("HOME is required")?;
                    registry::project::update(
                        Path::new(&config.project_registry_file),
                        &home,
                        Path::new(&config.projects_root),
                        &registry::project::SystemClock,
                    )?;
                    return Ok(());
                }
                ProjectCommand::Use { path } => {
                    let config = config::Config::load()?;
                    registry::project::use_project(
                        Path::new(&config.project_registry_file),
                        path.as_deref(),
                        Path::new(&config.projects_root),
                        &registry::project::SystemClock,
                    )?;
                    return Ok(());
                }
                ProjectCommand::Edit { path } => {
                    let config = config::Config::load()?;
                    registry::project::edit(Path::new(&config.project_registry_file), &path)?;
                    return Ok(());
                }
            },
            Command::Ssh { command } => {
                let config = config::Config::load()?;
                let registry = Path::new(&config.ssh_registry_file);
                let ssh_config = Path::new(&config.ssh_config_file);
                let history = Path::new(&config.ssh_history_file);
                match command {
                    SshCommand::Pick => {
                        let client = client
                            .as_ref()
                            .context("Herdr client is required for the SSH picker")?;
                        flows::picker::ssh_pick(registry, ssh_config, history, client)?;
                        return Ok(());
                    }
                    SshCommand::Sync => {
                        registry::ssh::sync(registry, ssh_config, history)?;
                        return Ok(());
                    }
                    SshCommand::List => {
                        use std::io::Write;
                        std::io::stdout()
                            .write_all(&registry::ssh::list(registry, ssh_config, history)?)?;
                        return Ok(());
                    }
                    SshCommand::Get { target } => {
                        print!(
                            "{}",
                            registry::ssh::get(registry, ssh_config, history, &target)?
                        );
                        return Ok(());
                    }
                    SshCommand::Use { target } => {
                        registry::ssh::use_target(registry, ssh_config, history, &target)?;
                        return Ok(());
                    }
                    SshCommand::Remove { target } => {
                        registry::ssh::remove(registry, ssh_config, history, &target)?;
                        return Ok(());
                    }
                    SshCommand::Edit { target } => {
                        flows::ssh::edit(target.as_deref(), ssh_config, registry, history)?;
                        return Ok(());
                    }
                    SshCommand::Session { target, tab_id } => {
                        let config = config::Config::load()?;
                        let home = std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .context("HOME is required")?;
                        let registry = Path::new(&config.ssh_registry_file);
                        let history_file = home.join(".zsh_history");
                        let client = client
                            .as_ref()
                            .context("Herdr client is required for an SSH session")?;
                        return flows::ssh::session(
                            &target,
                            &tab_id,
                            registry,
                            &history_file,
                            client,
                        );
                    }
                }
            }
            Command::Dashboard => {
                let config = config::Config::load().context("failed to load config")?;
                let client = SocketClient::new()?;
                return flows::dashboard::run(&client, &config);
            }
            Command::Config { command } => match command {
                ConfigCommand::Migrate { from, write, force } => {
                    if force && !write {
                        bail!("--force requires --write");
                    }

                    let partial_config =
                        config::migrate(from.as_deref()).context("failed to migrate zsh config")?;
                    let toml = config::serialize_migration(&partial_config)
                        .context("failed to create TOML config")?;
                    if !write {
                        print!("{toml}");
                        return Ok(());
                    }

                    let destination = config::resolved_config_path()
                        .context("failed to resolve config.toml destination")?;
                    let source = config::migration_source_path(from.as_deref())
                        .context("failed to resolve the migration source")?;
                    guard_write_destination(&destination, &source)?;
                    if destination
                        .try_exists()
                        .with_context(|| format!("failed to check {}", destination.display()))?
                        && !force
                    {
                        bail!("refusing to overwrite {}", destination.display());
                    }

                    write_atomically(&destination, toml.as_bytes())?;
                    println!("{}", destination.display());
                    return Ok(());
                }
            },
            Command::Herdr { command } => {
                return match command {
                    HerdrCommand::Ping => {
                        let response = SocketClient::new()?.ping()?;
                        println!("herdr {}, protocol {}", response.version, response.protocol);
                        Ok(())
                    }
                };
            }
        };

        Err(anyhow!("unimplemented: {path}"))
    }

    fn uses_herdr(&self) -> bool {
        !matches!(
            &self.command,
            Command::Project {
                command: ProjectCommand::Source { .. }
                    | ProjectCommand::Scan
                    | ProjectCommand::Rescan
                    | ProjectCommand::Update
                    | ProjectCommand::Use { .. }
                    | ProjectCommand::Edit { .. },
            } | Command::Ssh {
                command: SshCommand::Sync
                    | SshCommand::List
                    | SshCommand::Get { .. }
                    | SshCommand::Use { .. }
                    | SshCommand::Remove { .. }
                    | SshCommand::Edit { .. },
            } | Command::Config {
                command: ConfigCommand::Migrate { .. },
            }
        )
    }

    fn guard_protocol(&self, client: &dyn HerdrClient) -> Result<()> {
        match check_protocol(client) {
            Ok(()) => Ok(()),
            Err(ProtocolGuardError::Mismatch { expected, actual }) => {
                let message = format!(
                    "Herdr was upgraded from protocol {expected} to protocol {actual}. \
                     Rebuild this plugin for the new protocol."
                );
                notify::notify(client, "Workbench needs rebuilding", &message);
                Err(anyhow!(message))
            }
            Err(error @ ProtocolGuardError::Connection(_)) => {
                let message = format!("Could not connect to Herdr for the protocol check: {error}");
                notify::notify(client, "Workbench could not reach Herdr", &message);
                Err(anyhow!(message))
            }
        }
    }
}

/// Refuse a `--write` destination that would destroy a `config.zsh`.
///
/// The destination comes from `Q_WORKBENCH_LOCAL_CONFIG` when that is set, so it can
/// point anywhere — including at the very file being migrated. Writing TOML over the
/// source destroys the only copy of the settings, and `--force` alone would allow it.
/// Two guards close that: the destination may not be the source, and it must be named
/// `.toml`, which no `config.zsh` ever is.
fn guard_write_destination(destination: &Path, source: &Path) -> Result<()> {
    if destination.extension().and_then(|name| name.to_str()) != Some("toml") {
        bail!(
            "refusing to write TOML to a destination that is not a .toml file: {}",
            destination.display()
        );
    }

    // Compare resolved paths so a relative `--from` or a symlinked config directory
    // cannot slip past the check. An unresolvable path simply falls back to itself.
    let resolve = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if resolve(destination) == resolve(source) {
        bail!(
            "refusing to overwrite the migration source: {}",
            source.display()
        );
    }
    Ok(())
}

fn write_atomically(destination: &Path, contents: &[u8]) -> Result<()> {
    let file_name = destination
        .file_name()
        .with_context(|| format!("destination has no file name: {}", destination.display()))?;
    let temporary = destination.with_file_name(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    fs::write(&temporary, contents)
        .with_context(|| format!("failed to write temporary file {}", temporary.display()))?;
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to replace {}", destination.display()));
    }
    Ok(())
}

/// Serve `project source` without building clap's command tree.
///
/// The picker binds `change:reload(<self> source {q})`, so this subcommand runs once
/// per keystroke and process startup is its whole cost. Matching argv here skips
/// clap's parser construction for the one hot path; every other invocation returns
/// `None` and takes the normal route. A failure exits non-zero **silently**: the
/// picker must never gain output of any kind, on either channel (MSG-6).
fn project_source_fast_path() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != "project" || args.next()? != "source" {
        return None;
    }

    let query = match args.next() {
        None => None,
        // A dash means a flag, and non-UTF-8 is a value clap rejects. Both are rare
        // enough to hand back to clap rather than reimplement.
        Some(value) => match value.into_string() {
            Ok(value) if !value.starts_with('-') => Some(value),
            _ => return None,
        },
    };
    if args.next().is_some() {
        return None;
    }

    Some(match flows::picker::project_source(query.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    })
}

fn main() -> ExitCode {
    if let Some(code) = project_source_fast_path() {
        return code;
    }

    match Cli::parse().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{error:#}` prints the whole `.context()` chain on one line. Plain
            // `{error}` shows only the outermost message, which drops the concrete
            // cause every fatal path is required to report.
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_leaf_parses_with_all_supported_arguments() {
        let cases = [
            vec!["workbench", "agent", "popup", "--worktree"],
            vec![
                "workbench",
                "agent",
                "launch",
                "w1:p1",
                "--tab",
                "tab-1",
                "--usage",
                "test",
                "--worktree",
                "--no-layout",
                "--restart",
            ],
            vec![
                "workbench",
                "agent",
                "inject",
                "w1:p1",
                "--tab",
                "tab-1",
                "--usage",
                "test",
                "--worktree",
            ],
            vec!["workbench", "agent", "restart"],
            vec!["workbench", "agent", "restart-worker", "--pane", "w1:p1"],
            vec!["workbench", "project", "pick"],
            vec!["workbench", "project", "source", "query"],
            vec!["workbench", "project", "scan"],
            vec!["workbench", "project", "rescan"],
            vec!["workbench", "project", "update"],
            vec!["workbench", "project", "use", "/tmp/project"],
            vec!["workbench", "project", "edit", "/tmp/project"],
            vec!["workbench", "ssh", "pick"],
            vec!["workbench", "ssh", "sync"],
            vec!["workbench", "ssh", "list"],
            vec!["workbench", "ssh", "get", "host"],
            vec!["workbench", "ssh", "use", "host"],
            vec!["workbench", "ssh", "remove", "host"],
            vec!["workbench", "ssh", "edit", "host"],
            vec!["workbench", "ssh", "session", "host", "tab-1"],
            vec!["workbench", "dashboard"],
            vec![
                "workbench",
                "config",
                "migrate",
                "--from",
                "/tmp/config",
                "--write",
                "--force",
            ],
            vec!["workbench", "herdr", "ping"],
        ];

        for argv in cases {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "failed to parse {argv:?}"
            );
        }
    }

    #[test]
    fn invalid_inputs_return_clap_errors() {
        let cases = [
            vec!["workbench", "unknown"],
            vec!["workbench", "agent", "popup", "--unknown"],
            vec!["workbench", "agent", "launch"],
        ];

        for argv in cases {
            assert!(
                Cli::try_parse_from(&argv).is_err(),
                "parsed invalid {argv:?}"
            );
        }
    }

    #[test]
    fn ssh_edit_accepts_a_missing_target() {
        assert!(Cli::try_parse_from(["workbench", "ssh", "edit"]).is_ok());
    }

    #[test]
    fn protocol_mismatch_sends_exactly_one_notification_with_both_numbers() {
        let cli = Cli::try_parse_from(["workbench", "dashboard"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_response(
            "ping",
            serde_json::json!({
                "type": "ping",
                "version": "2.0.0",
                "protocol": herdr::EXPECTED_PROTOCOL + 1,
            }),
        );

        let error = cli.guard_protocol(&client).unwrap_err().to_string();
        let calls = client.calls.into_inner();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "ping");
        assert_eq!(calls[1].0, "notification.show");
        let body = calls[1].1["body"].as_str().unwrap();
        assert!(body.contains(&herdr::EXPECTED_PROTOCOL.to_string()));
        assert!(body.contains(&(herdr::EXPECTED_PROTOCOL + 1).to_string()));
        assert!(error.contains("Rebuild this plugin"));
    }

    #[test]
    fn protocol_connection_failure_has_a_distinct_message() {
        let cli = Cli::try_parse_from(["workbench", "dashboard"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_error("ping", "unavailable", "socket unavailable");

        let error = cli.guard_protocol(&client).unwrap_err().to_string();

        assert!(error.contains("Could not connect to Herdr"));
        assert!(!error.contains("protocol mismatch"));
    }

    #[test]
    fn every_local_only_subcommand_skips_ping() {
        let cases = [
            vec!["workbench", "project", "source", "query"],
            vec!["workbench", "project", "scan"],
            vec!["workbench", "project", "rescan"],
            vec!["workbench", "project", "update"],
            vec!["workbench", "project", "use", "/tmp/project"],
            vec!["workbench", "project", "edit", "/tmp/project"],
            vec!["workbench", "ssh", "sync"],
            vec!["workbench", "ssh", "list"],
            vec!["workbench", "ssh", "get", "host"],
            vec!["workbench", "ssh", "use", "host"],
            vec!["workbench", "ssh", "remove", "host"],
            vec!["workbench", "ssh", "edit", "host"],
            vec!["workbench", "config", "migrate"],
        ];

        for argv in cases {
            let cli = Cli::try_parse_from(&argv).unwrap();
            let client = herdr::FakeClient::default();

            if cli.uses_herdr() {
                cli.guard_protocol(&client).unwrap();
            }

            assert!(
                client.calls.borrow().is_empty(),
                "{argv:?} unexpectedly called Herdr"
            );
        }
    }
}
