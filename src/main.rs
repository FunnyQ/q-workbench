use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand};

#[allow(dead_code)]
mod herdr;
#[allow(dead_code)]
mod config;

use herdr::{HerdrClient, SocketClient};

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
    Use { path: PathBuf },
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
    Migrate {
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,
        #[arg(long)]
        write: bool,
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
            Command::Config { command } => match command {
                ConfigCommand::Migrate { .. } => "config migrate",
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
}

fn main() -> ExitCode {
    match Cli::parse().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
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
}
