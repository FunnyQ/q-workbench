use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};

// The binary is a thin shell over the library target: declaring the modules here too
// would compile every one of them a second time and force a blanket dead-code allow,
// because each copy sees only the callers in its own target.
use workbench::flows::{FlowError, FlowResult, Outcome};
use workbench::herdr::{check_protocol, HerdrClient, ProtocolGuardError, SocketClient};
use workbench::{config, flows, notify, registry};

/// Routes failures either to a durable terminal or to a popup notification.
/// The route also records whether the command needs Herdr, so setup cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Notification(&'static str),
    Stderr { uses_herdr: bool },
}

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
    Herdr {
        #[command(subcommand)]
        command: HerdrCommand,
    },
    Pane {
        #[command(subcommand)]
        command: PaneCommand,
    },
    Tab {
        #[command(subcommand)]
        command: TabCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    Popup {
        #[arg(long)]
        worktree: bool,
        #[arg(long)]
        layout: Option<String>,
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
    #[arg(long)]
    layout: Option<String>,
    /// Which agent pane of the layout to launch. Defaults to the layout's first.
    #[arg(long)]
    pane: Option<String>,
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
    #[arg(long)]
    layout: Option<String>,
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
enum HerdrCommand {
    Ping,
}

#[derive(Debug, Subcommand)]
enum PaneCommand {
    /// Even out the split ratios in the current pane's row or column.
    Even {
        #[arg(long)]
        pane: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum TabCommand {
    /// Pick a tab layout, then open the agent popup for it.
    New,
}

impl Cli {
    /// Classify parsed commands against the fixed parity-contract lists.
    /// Command identity, rather than error text, determines the reporting surface.
    fn channel(&self) -> Channel {
        match &self.command {
            Command::Agent {
                command: AgentCommand::Popup { .. },
            } => Channel::Notification("Agent popup failed"),
            Command::Agent {
                command: AgentCommand::Launch(_),
            } => Channel::Notification("Agent launch failed"),
            Command::Agent {
                command: AgentCommand::Inject(_),
            } => Channel::Notification("Agent inject failed"),
            Command::Agent {
                command: AgentCommand::Restart | AgentCommand::RestartWorker { .. },
            } => Channel::Notification("Agent restart failed"),
            Command::Project {
                command: ProjectCommand::Pick,
            } => Channel::Notification("Project picker"),
            Command::Ssh {
                command: SshCommand::Pick,
            } => Channel::Notification("SSH picker"),
            Command::Ssh {
                command: SshCommand::Session { .. },
            } => Channel::Notification("SSH session"),
            Command::Dashboard => Channel::Notification("Dashboard Launcher"),
            Command::Project {
                command:
                    ProjectCommand::Source { .. }
                    | ProjectCommand::Scan
                    | ProjectCommand::Rescan
                    | ProjectCommand::Update
                    | ProjectCommand::Use { .. }
                    | ProjectCommand::Edit { .. },
            }
            | Command::Ssh {
                command:
                    SshCommand::Sync
                    | SshCommand::List
                    | SshCommand::Get { .. }
                    | SshCommand::Use { .. }
                    | SshCommand::Remove { .. },
            }
            | Command::Ssh {
                command: SshCommand::Edit { .. },
            } => Channel::Stderr { uses_herdr: false },
            Command::Herdr {
                command: HerdrCommand::Ping,
            } => Channel::Stderr { uses_herdr: true },
            Command::Pane {
                command: PaneCommand::Even { .. },
            } => Channel::Notification("Even out panes failed"),
            Command::Tab {
                command: TabCommand::New,
            } => Channel::Notification("New tab"),
        }
    }

    fn subcommand_path(&self) -> &'static str {
        match &self.command {
            Command::Agent { command } => match command {
                AgentCommand::Popup { .. } => "agent popup",
                AgentCommand::Launch(_) => "agent launch",
                AgentCommand::Inject(_) => "agent inject",
                AgentCommand::Restart => "agent restart",
                AgentCommand::RestartWorker { .. } => "agent restart-worker",
            },
            Command::Project { command } => match command {
                ProjectCommand::Pick => "project pick",
                ProjectCommand::Source { .. } => "project source",
                ProjectCommand::Scan => "project scan",
                ProjectCommand::Rescan => "project rescan",
                ProjectCommand::Update => "project update",
                ProjectCommand::Use { .. } => "project use",
                ProjectCommand::Edit { .. } => "project edit",
            },
            Command::Ssh { command } => match command {
                SshCommand::Pick => "ssh pick",
                SshCommand::Sync => "ssh sync",
                SshCommand::List => "ssh list",
                SshCommand::Get { .. } => "ssh get",
                SshCommand::Use { .. } => "ssh use",
                SshCommand::Remove { .. } => "ssh remove",
                SshCommand::Edit { .. } => "ssh edit",
                SshCommand::Session { .. } => "ssh session",
            },
            Command::Dashboard => "dashboard",
            Command::Herdr { .. } => "herdr ping",
            Command::Pane { command } => match command {
                PaneCommand::Even { .. } => "pane even",
            },
            Command::Tab { command } => match command {
                TabCommand::New => "tab new",
            },
        }
    }

    #[allow(clippy::needless_return)]
    fn run(self, client: Option<&dyn HerdrClient>) -> FlowResult {
        match self.command {
            Command::Agent { command } => match command {
                AgentCommand::Popup { worktree, layout } => {
                    let client = client.context("Herdr client is required for agent popup")?;
                    return flows::agent::popup(client, worktree, layout.as_deref());
                }
                AgentCommand::Launch(args) => {
                    let client = client.context("Herdr client is required for agent launch")?;
                    let config = config::Config::load().context("failed to load config")?;
                    return flows::agent::launch(
                        client,
                        &config,
                        &flows::agent::LaunchOptions {
                            pane_id: args.pane_id,
                            tab_id: args.tab,
                            usage: args.usage,
                            worktree: args.worktree,
                            no_layout: args.no_layout,
                            restart: args.restart,
                            layout: args.layout,
                            pane: args.pane,
                        },
                    );
                }
                AgentCommand::Inject(args) => {
                    let client = client.context("Herdr client is required for agent inject")?;
                    return flows::agent::inject(
                        client,
                        &flows::agent::InjectOptions {
                            pane_id: args.pane_id,
                            tab_id: args.tab,
                            usage: args.usage,
                            worktree: args.worktree,
                            layout: args.layout,
                        },
                    );
                }
                AgentCommand::Restart => {
                    let client = client.context("Herdr client is required for agent restart")?;
                    return flows::restart::confirm_restart(client);
                }
                AgentCommand::RestartWorker { pane } => {
                    let client =
                        client.context("Herdr client is required for agent restart worker")?;
                    return flows::restart::restart_worker(client, &pane);
                }
            },
            Command::Project { command } => match command {
                ProjectCommand::Pick => {
                    let config = config::Config::load()?;
                    let client =
                        client.context("Herdr client is required for the project picker")?;
                    return flows::picker::project_pick(
                        &config,
                        Path::new(&config.project_registry_file),
                        Path::new(&config.projects_root),
                        client,
                    );
                }
                ProjectCommand::Source { query } => {
                    flows::picker::project_source(query.as_deref())?;
                    return Ok(Outcome::Done);
                }
                ProjectCommand::Scan => {
                    let config = config::Config::load()?;
                    registry::project::scan(Path::new(&config.project_registry_file))?;
                    return Ok(Outcome::Done);
                }
                ProjectCommand::Rescan => {
                    let config = config::Config::load()?;
                    registry::project::rescan(Path::new(&config.project_registry_file))?;
                    return Ok(Outcome::Done);
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
                    return Ok(Outcome::Done);
                }
                ProjectCommand::Use { path } => {
                    let config = config::Config::load()?;
                    registry::project::use_project(
                        Path::new(&config.project_registry_file),
                        path.as_deref(),
                        Path::new(&config.projects_root),
                        &registry::project::SystemClock,
                    )?;
                    return Ok(Outcome::Done);
                }
                ProjectCommand::Edit { path } => {
                    let config = config::Config::load()?;
                    registry::project::edit(Path::new(&config.project_registry_file), &path)?;
                    return Ok(Outcome::Done);
                }
            },
            Command::Ssh { command } => {
                let config = config::Config::load()?;
                let registry = Path::new(&config.ssh_registry_file);
                let ssh_config = Path::new(&config.ssh_config_file);
                let history = Path::new(&config.ssh_history_file);
                match command {
                    SshCommand::Pick => {
                        let client =
                            client.context("Herdr client is required for the SSH picker")?;
                        return flows::picker::ssh_pick(registry, ssh_config, history, client);
                    }
                    SshCommand::Sync => {
                        registry::ssh::sync(registry, ssh_config, history)?;
                        return Ok(Outcome::Done);
                    }
                    SshCommand::List => {
                        use std::io::Write;
                        std::io::stdout()
                            .write_all(&registry::ssh::list(registry, ssh_config, history)?)?;
                        return Ok(Outcome::Done);
                    }
                    SshCommand::Get { target } => {
                        print!(
                            "{}",
                            registry::ssh::get(registry, ssh_config, history, &target)?
                        );
                        return Ok(Outcome::Done);
                    }
                    SshCommand::Use { target } => {
                        registry::ssh::use_target(registry, ssh_config, history, &target)?;
                        return Ok(Outcome::Done);
                    }
                    SshCommand::Remove { target } => {
                        registry::ssh::remove(registry, ssh_config, history, &target)?;
                        return Ok(Outcome::Done);
                    }
                    SshCommand::Edit { target } => {
                        flows::ssh::edit(target.as_deref(), ssh_config, registry, history)?;
                        return Ok(Outcome::Done);
                    }
                    SshCommand::Session { target, tab_id } => {
                        // The zsh session script appended to `$HOME/.zsh_history`
                        // literally, not to `$Q_SSH_HISTORY_FILE`; parity keeps that.
                        let home = std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .context("HOME is required")?;
                        let history_file = home.join(".zsh_history");
                        let client =
                            client.context("Herdr client is required for an SSH session")?;
                        return flows::ssh::session(
                            &target,
                            &tab_id,
                            registry,
                            ssh_config,
                            &history_file,
                            client,
                        );
                    }
                }
            }
            Command::Dashboard => {
                let config = config::Config::load().context("failed to load config")?;
                let client = client.context("Herdr client is required for the dashboard")?;
                return flows::dashboard::run(client, &config);
            }
            Command::Herdr { command } => {
                return match command {
                    HerdrCommand::Ping => {
                        let response = client
                            .context("Herdr client is required for ping")?
                            .ping()?;
                        println!("herdr {}, protocol {}", response.version, response.protocol);
                        Ok(Outcome::Done)
                    }
                };
            }
            Command::Pane { command } => match command {
                PaneCommand::Even { pane } => {
                    let client = client.context("Herdr client is required for pane even")?;
                    return flows::layout::even_out(client, pane.as_deref());
                }
            },
            Command::Tab { command } => match command {
                TabCommand::New => {
                    let client = client.context("Herdr client is required for a new tab")?;
                    return flows::tab::new(client);
                }
            },
        }
    }

    fn uses_herdr(&self) -> bool {
        match self.channel() {
            Channel::Notification(_) => true,
            Channel::Stderr { uses_herdr } => uses_herdr,
        }
    }

    fn notification_title(&self) -> Option<&'static str> {
        match self.channel() {
            Channel::Notification(title) => Some(title),
            Channel::Stderr { .. } => None,
        }
    }

    fn guard_protocol(&self, client: &dyn HerdrClient) -> Result<()> {
        match check_protocol(client) {
            Ok(()) => Ok(()),
            Err(ProtocolGuardError::TooOld { minimum, actual }) => {
                let message = format!(
                    "Herdr speaks protocol {actual}, but this plugin needs protocol {minimum} \
                     or newer. Update Herdr."
                );
                if matches!(self.channel(), Channel::Notification(_)) {
                    notify::notify(client, "Workbench needs a newer Herdr", &message);
                }
                Err(anyhow!(message))
            }
            Err(error @ ProtocolGuardError::Connection(_)) => {
                let message = format!("Could not connect to Herdr for the protocol check: {error}");
                if matches!(self.channel(), Channel::Notification(_)) {
                    notify::notify(client, "Workbench could not reach Herdr", &message);
                }
                Err(anyhow!(message))
            }
        }
    }
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

    let cli = Cli::parse();
    let channel = cli.channel();
    let subcommand_path = cli.subcommand_path();
    let notification_title = cli.notification_title();
    let client = if cli.uses_herdr() {
        match SocketClient::new() {
            Ok(client) => Some(client),
            Err(error) => {
                if matches!(channel, Channel::Stderr { .. }) {
                    report_stderr(subcommand_path, &error, &mut std::io::stderr());
                }
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    if cli.uses_herdr() {
        let client = client.as_ref().expect("Herdr commands create a client");
        if let Err(error) = cli.guard_protocol(client) {
            if matches!(channel, Channel::Stderr { .. }) {
                report_stderr(subcommand_path, &error, &mut std::io::stderr());
            }
            return ExitCode::FAILURE;
        }
    }

    let result = cli.run(client.as_ref().map(|client| client as &dyn HerdrClient));
    if matches!(channel, Channel::Notification(_)) {
        // No second connect attempt: `Channel::Notification` implies `uses_herdr`, so a
        // client was either built above or the run already returned FAILURE.
        if let (Some(default_title), Some(client)) = (notification_title, &client) {
            return handle_flow_result(client, default_title, result);
        }
    }

    match result {
        Ok(Outcome::Done | Outcome::Cancelled) => ExitCode::SUCCESS,
        Ok(Outcome::Notice { title, body }) => {
            if let Some(client) = &client {
                notify::notify(client, &title, &body);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            report_stderr(subcommand_path, &error, &mut std::io::stderr());
            ExitCode::FAILURE
        }
    }
}

fn report_stderr(subcommand_path: &str, error: &anyhow::Error, output: &mut impl std::io::Write) {
    let message = format!("{error:#}");
    // Contract messages already name their reporting surface. Other fatal errors use
    // the parsed clap path once, with chained causes from anyhow's `{:#}` format.
    let is_contract_message = subcommand_path == "ssh edit"
        && [
            "No SSH target selected.",
            "Invalid SSH alias: ",
            "Invalid HostName: ",
            "Invalid SSH user: ",
            "Invalid SSH port: ",
            "SSH alias already exists: ",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix));
    if message.starts_with(&format!("{subcommand_path}: "))
        || message.starts_with("project-registry: ")
        || is_contract_message
    {
        let _ = writeln!(output, "{message}");
    } else {
        let _ = writeln!(output, "{subcommand_path}: {message}");
    }
}

fn handle_flow_result(
    client: &dyn HerdrClient,
    default_title: &str,
    result: FlowResult,
) -> ExitCode {
    match result {
        Ok(Outcome::Done | Outcome::Cancelled) => ExitCode::SUCCESS,
        Ok(Outcome::Notice { title, body }) => {
            notify::notify(client, &title, &body);
            ExitCode::SUCCESS
        }
        Err(error) => {
            report_flow_error(client, default_title, &error);
            ExitCode::FAILURE
        }
    }
}

fn report_flow_error(client: &dyn HerdrClient, default_title: &str, error: &anyhow::Error) {
    let metadata = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<FlowError>());
    let title = metadata.and_then(FlowError::title).unwrap_or(default_title);
    let body = notification_body(metadata, error);
    notify::notify(client, title, &body.replace(['\r', '\n'], " "));
}

/// Build the one-line body a failed flow reports.
///
/// A `FlowError` prefix is a sentence the flow wants kept verbatim at the front of the
/// body, with the chained cause after it — the agent popup's
/// `The incomplete tab was closed.` describes the cleanup rather than the failure, so it
/// needs the cause to say what went wrong.
///
/// `FlowError::complete` is the other shape, used by the dashboard's missing workspace
/// and the project picker's two contract messages. Those sentences already name their
/// concrete cause, so the flow stores the same sentence as both the prefix and the
/// error; the equality below recognises that and prints it once instead of twice.
fn notification_body(metadata: Option<&FlowError>, error: &anyhow::Error) -> String {
    let chain = metadata
        .map(FlowError::chain)
        .unwrap_or_else(|| format!("{error:#}"));
    match metadata.and_then(FlowError::prefix) {
        Some(prefix) if prefix == chain => chain,
        Some(prefix) => format!("{prefix} {chain}"),
        None => chain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbench::herdr;

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
            vec!["workbench", "herdr", "ping"],
            vec!["workbench", "tab", "new"],
        ];

        for argv in cases {
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "failed to parse {argv:?}"
            );
        }
    }

    #[test]
    fn agent_result_reports_notice_error_and_failed_delivery() {
        let notice = herdr::FakeClient::default();
        assert_eq!(
            handle_flow_result(
                &notice,
                "unused",
                Ok(Outcome::Notice {
                    title: "Restart agent".to_owned(),
                    body: "No agent pane in this tab to restart.".to_owned(),
                }),
            ),
            ExitCode::SUCCESS
        );
        assert_eq!(notice.calls.borrow().len(), 1);
        assert_eq!(notice.calls.borrow()[0].1["title"], "Restart agent");
        assert_eq!(
            notice.calls.borrow()[0].1["body"],
            "No agent pane in this tab to restart."
        );

        let failure = herdr::FakeClient::default();
        failure.queue_error("notification.show", "unavailable", "socket gone");
        let error = FlowError::prefixed(
            "Agent tab failed",
            "The incomplete tab was closed.",
            anyhow!("pane.split failed: connection refused"),
        );
        assert_eq!(
            handle_flow_result(&failure, "Agent popup failed", Err(error.into())),
            ExitCode::FAILURE
        );
        assert_eq!(failure.calls.borrow().len(), 1);
        let body = failure.calls.borrow()[0].1["body"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(body.starts_with("The incomplete tab was closed."));
        assert!(body.contains("pane.split failed: connection refused"));
        assert!(!body.contains('\n'));
    }

    /// The four non-agent notifying subcommands must carry the titles the parity
    /// contract names, because `notification_title` is what a failure with no title of
    /// its own falls back to.
    #[test]
    fn non_agent_notifying_subcommands_carry_their_contract_titles() {
        let cases = [
            (vec!["workbench", "project", "pick"], Some("Project picker")),
            (vec!["workbench", "ssh", "pick"], Some("SSH picker")),
            (
                vec!["workbench", "ssh", "session", "host", "t1"],
                Some("SSH session"),
            ),
            (vec!["workbench", "dashboard"], Some("Dashboard Launcher")),
            (vec!["workbench", "tab", "new"], Some("New tab")),
            // Terminal-facing subcommands report on stderr instead.
            (vec!["workbench", "ssh", "list"], None),
            (vec!["workbench", "project", "scan"], None),
        ];

        for (argv, expected) in cases {
            let cli = Cli::try_parse_from(&argv).unwrap();
            assert_eq!(cli.notification_title(), expected, "{argv:?}");
        }
    }

    #[test]
    fn every_subcommand_selects_its_fixed_channel() {
        let cases = [
            (
                vec!["workbench", "project", "pick"],
                Channel::Notification("Project picker"),
            ),
            (
                vec!["workbench", "project", "scan"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "project", "rescan"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "project", "update"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "project", "use"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "project", "edit", "/tmp/p"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "project", "source"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "sync"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "list"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "get", "host"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "use", "host"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "remove", "host"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "ssh", "edit"],
                Channel::Stderr { uses_herdr: false },
            ),
            (
                vec!["workbench", "herdr", "ping"],
                Channel::Stderr { uses_herdr: true },
            ),
            (
                vec!["workbench", "tab", "new"],
                Channel::Notification("New tab"),
            ),
        ];

        for (argv, expected) in cases {
            let cli = Cli::try_parse_from(&argv).unwrap();
            assert_eq!(cli.channel(), expected, "{argv:?}");
            if argv.get(1) == Some(&"project") && argv.get(2) != Some(&"pick") {
                assert_eq!(cli.notification_title(), None, "{argv:?}");
            }
        }
    }

    #[test]
    fn stderr_preserves_contract_messages_and_prefixes_unnamed_failures() {
        let mut contract = Vec::new();
        report_stderr(
            "project scan",
            &anyhow!("project-registry: no projects found"),
            &mut contract,
        );
        assert_eq!(contract, b"project-registry: no projects found\n");

        let mut unnamed = Vec::new();
        report_stderr(
            "project update",
            &anyhow!("disk full").context("failed to write registry"),
            &mut unnamed,
        );
        assert_eq!(
            unnamed,
            b"project update: failed to write registry: disk full\n"
        );

        for message in [
            "No SSH target selected.",
            "Invalid SSH alias: bad alias",
            "Invalid HostName: bad host",
            "Invalid SSH user: bad user",
            "Invalid SSH port: 0",
            "SSH alias already exists: host",
        ] {
            let mut output = Vec::new();
            report_stderr("ssh edit", &anyhow!(message), &mut output);
            assert_eq!(output, format!("{message}\n").as_bytes());
        }

        for subcommand in ["ssh sync", "herdr ping"] {
            let mut output = Vec::new();
            report_stderr(subcommand, &anyhow!("disk full"), &mut output);
            assert_eq!(output, format!("{subcommand}: disk full\n").as_bytes());
        }
    }

    /// A body built by `FlowError::complete` reaches the notification whole, with no
    /// chained cause appended — this is the dashboard's and the project picker's
    /// contract, driven end to end rather than asserted on the metadata.
    #[test]
    fn a_complete_body_is_reported_verbatim_with_nothing_appended() {
        let cases = [
            (
                "Dashboard Launcher",
                "Workspace 'personal-assistant' was not found.",
            ),
            (
                "Project picker",
                "project picker: registry not found: /state/registry.json",
            ),
            (
                "Project picker",
                "project picker: project not found: nowhere",
            ),
        ];

        for (title, body) in cases {
            let client = herdr::FakeClient::default();
            let error = FlowError::complete(title, body);
            assert_eq!(
                handle_flow_result(&client, "unused default", Err(error.into())),
                ExitCode::FAILURE
            );

            let calls = client.calls.into_inner();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, "notification.show");
            assert_eq!(calls[0].1["title"], title);
            assert_eq!(calls[0].1["body"], body);
        }
    }

    /// A failure with no preserved sentence reports the chained cause under the
    /// subcommand's own title.
    #[test]
    fn a_titled_failure_reports_its_chain_under_its_own_title() {
        let client = herdr::FakeClient::default();
        let error = FlowError::titled(
            "SSH picker",
            anyhow!("ssh pick: tab.create").context("Herdr error unavailable: socket closed"),
        );

        assert_eq!(
            handle_flow_result(&client, "unused default", Err(error.into())),
            ExitCode::FAILURE
        );

        let calls = client.calls.into_inner();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["title"], "SSH picker");
        assert_eq!(
            calls[0].1["body"],
            "Herdr error unavailable: socket closed: ssh pick: tab.create"
        );
    }

    #[test]
    fn agent_cancellation_is_silent() {
        let client = herdr::FakeClient::default();
        assert_eq!(
            handle_flow_result(&client, "Agent popup failed", Ok(Outcome::Cancelled)),
            ExitCode::SUCCESS
        );
        assert!(client.calls.borrow().is_empty());
    }

    #[test]
    fn restart_worker_reports_resolve_kill_and_reinject_failures_once() {
        let clients = [
            {
                let client = herdr::FakeClient::default();
                client.queue_error("pane.get", "unavailable", "resolve failed");
                client
            },
            {
                let client = herdr::FakeClient::default();
                client.queue_response(
                    "pane.get",
                    serde_json::json!({"pane": {
                        "pane_id": "p1", "tab_id": "t1", "agent": {}, "label": "review"
                    }}),
                );
                client.queue_response(
                    "pane.process_info",
                    serde_json::json!({"process_info": {
                        "foreground_process_group_id": i32::MAX,
                        "shell_pid": 1
                    }}),
                );
                client
            },
            {
                let client = herdr::FakeClient::default();
                client.queue_response(
                    "pane.get",
                    serde_json::json!({"pane": {
                        "pane_id": "p1", "tab_id": "t1", "agent": {}, "label": "review"
                    }}),
                );
                client.queue_response(
                    "pane.process_info",
                    serde_json::json!({"process_info": null}),
                );
                client.queue_error("pane.send_input", "unavailable", "reinject failed");
                client
            },
        ];

        for client in clients {
            let result = flows::restart::restart_worker(&client, "p1");
            assert_eq!(
                handle_flow_result(&client, "Agent restart failed", result),
                ExitCode::FAILURE
            );
            let calls = client.calls.borrow();
            let notifications = calls
                .iter()
                .filter(|call| call.0 == "notification.show")
                .collect::<Vec<_>>();
            assert_eq!(notifications.len(), 1);
            let body = notifications[0].1["body"].as_str().unwrap();
            assert!(!body.is_empty());
            assert_ne!(body, "error");
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
    fn stale_protocol_sends_exactly_one_notification_with_both_numbers() {
        let cli = Cli::try_parse_from(["workbench", "dashboard"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_response(
            "ping",
            serde_json::json!({
                "type": "ping",
                "version": "2.0.0",
                "protocol": herdr::MINIMUM_PROTOCOL - 1,
            }),
        );

        let error = cli.guard_protocol(&client).unwrap_err().to_string();
        let calls = client.calls.into_inner();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "ping");
        assert_eq!(calls[1].0, "notification.show");
        let body = calls[1].1["body"].as_str().unwrap();
        assert!(body.contains(&herdr::MINIMUM_PROTOCOL.to_string()));
        assert!(body.contains(&(herdr::MINIMUM_PROTOCOL - 1).to_string()));
        assert!(error.contains("Update Herdr"));
    }

    #[test]
    fn a_newer_protocol_passes_the_guard_without_notifying() {
        let cli = Cli::try_parse_from(["workbench", "dashboard"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_response(
            "ping",
            serde_json::json!({
                "type": "ping",
                "version": "9.0.0",
                "protocol": herdr::MINIMUM_PROTOCOL + 5,
            }),
        );

        cli.guard_protocol(&client).unwrap();

        assert_eq!(client.calls.into_inner().len(), 1);
    }

    #[test]
    fn protocol_connection_failure_has_a_distinct_message() {
        let cli = Cli::try_parse_from(["workbench", "dashboard"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_error("ping", "unavailable", "socket unavailable");

        let error = cli.guard_protocol(&client).unwrap_err().to_string();

        assert!(error.contains("Could not connect to Herdr"));
        assert!(!error.contains("Update Herdr"));
    }

    #[test]
    fn herdr_ping_protocol_failure_never_notifies() {
        let cli = Cli::try_parse_from(["workbench", "herdr", "ping"]).unwrap();
        let client = herdr::FakeClient::default();
        client.queue_error("ping", "unavailable", "socket unavailable");

        cli.guard_protocol(&client).unwrap_err();

        assert_eq!(client.calls.borrow().len(), 1);
        assert_eq!(client.calls.borrow()[0].0, "ping");
    }

    #[test]
    fn project_subcommands_never_call_notification_show() {
        let cases = [
            vec!["workbench", "project", "source", "query"],
            vec!["workbench", "project", "scan"],
            vec!["workbench", "project", "rescan"],
            vec!["workbench", "project", "update"],
            vec!["workbench", "project", "use", "/tmp/project"],
            vec!["workbench", "project", "edit", "/tmp/project"],
        ];

        for argv in cases {
            let cli = Cli::try_parse_from(&argv).unwrap();
            let client = herdr::FakeClient::default();

            if cli.uses_herdr() {
                cli.guard_protocol(&client).unwrap();
            }

            assert!(
                client
                    .calls
                    .borrow()
                    .iter()
                    .all(|call| call.0 != "notification.show"),
                "{argv:?} unexpectedly issued notification.show"
            );
        }
    }

    #[test]
    fn terminal_subcommands_never_call_notification_show() {
        let cases = [
            vec!["workbench", "ssh", "sync"],
            vec!["workbench", "ssh", "list"],
            vec!["workbench", "ssh", "get", "host"],
            vec!["workbench", "ssh", "use", "host"],
            vec!["workbench", "ssh", "remove", "host"],
            vec!["workbench", "ssh", "edit", "host"],
            vec!["workbench", "herdr", "ping"],
        ];

        for argv in cases {
            let cli = Cli::try_parse_from(&argv).unwrap();
            let client = herdr::FakeClient::default();
            client.queue_error("ping", "unavailable", "socket unavailable");

            if cli.uses_herdr() {
                let _ = cli.guard_protocol(&client);
            }

            assert!(
                client
                    .calls
                    .borrow()
                    .iter()
                    .all(|call| call.0 != "notification.show"),
                "{argv:?} unexpectedly issued notification.show"
            );
        }
    }
}
