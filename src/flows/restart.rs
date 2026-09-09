use std::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::config::Config;
use crate::flows::menu::{popup_viewport, strip_pad, GumMenu, Menu};
use crate::flows::session;
use crate::flows::{FlowError, FlowResult, Outcome};
use crate::herdr::types::Pane;
use crate::herdr::HerdrClient;
use crate::shell::build_command;
use crate::state;

const NO_AGENT: &str = "No agent pane in this tab to restart.";
const CANNOT_FOCUS: &str = "Could not focus the agent pane.";
const NO_SESSION: &str = "No session to resume was recorded, so the agent started fresh.";
const CANNOT_RESUME: &str = "This agent cannot resume a session, so it started fresh.";
const NOTIFICATION_TITLE: &str = "Restart agent";
const FAILURE_TITLE: &str = "Agent restart failed";
const MENU_TITLE: &str = "\u{f002a}  Restart Agent";
const MENU_SUBTITLE: &str = "The agent will relaunch in place.";
const MENU_HEIGHT: u8 = 8;
const RESUME_OPTION: &str = "\u{f0709}  resume this session";
const FRESH_OPTION: &str = "\u{ec58}  start fresh";
const CANCEL_OPTION: &str = "\u{ea76}  cancel";

// Codex leaves raw mode and Kitty CSI-u enabled, breaking line wrapping and menu arrow keys.
// The detached worker cannot access the pane TTY, so this prefix must run inside the pane.
// Keep the prefix unquoted for shell interpretation; quote only the launcher path and arguments.
const TTY_RESET: &str = "stty sane; printf '\\033[<u\\033[?7h\\033[?25h\\033[0m'; ";

/// What the restart menu decided. Cancelling is `None`, never a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartMode {
    Resume,
    Fresh,
}

/// Confirms the restart inside the popup, then hands the work to a detached worker.
///
/// The popup pane disappears the moment this returns, so nothing here may resolve the
/// target or touch the agent: the worker must do all of it. What the worker receives is
/// the **invocation** pane — the pane the action fired from, which is frequently the
/// yazi or term pane. Handing it the agent pane instead would make the worker believe
/// focus is already correct and skip the focus walk entirely.
pub fn confirm_restart(_client: &dyn HerdrClient) -> FlowResult {
    let invocation_pane_id =
        invocation_pane_id().map_err(|error| FlowError::titled(FAILURE_TITLE, error))?;

    let executable = env::current_exe().context("failed to resolve the workbench executable")?;
    let spawned = confirm_and_spawn(&invocation_pane_id, &run_confirm, &|pane_id, mode| {
        spawn_worker(&executable, pane_id, mode).map(|_| ())
    })
    .map_err(|error| FlowError::titled(FAILURE_TITLE, error))?;
    Ok(if spawned.is_some() {
        Outcome::Done
    } else {
        Outcome::Cancelled
    })
}

/// The decision half of `confirm_restart`, with the two side effects injected.
///
/// It deliberately holds no Herdr client: proving that this phase talks to nobody is
/// what guarantees the worker — not the popup — resolves the target.
fn confirm_and_spawn(
    invocation_pane_id: &str,
    confirm: &dyn Fn() -> Result<Option<RestartMode>>,
    spawn: &dyn Fn(&str, RestartMode) -> Result<()>,
) -> Result<Option<RestartMode>> {
    let Some(mode) = confirm()? else {
        return Ok(None);
    };
    spawn(invocation_pane_id, mode)?;
    Ok(Some(mode))
}

/// Restarts the agent pane reachable from `invocation_pane_id`.
///
/// Runs in the detached worker, so `invocation_pane_id` is the pane the popup was
/// opened from and may well hold yazi or a shell rather than the agent.
pub fn restart_worker(
    client: &dyn HerdrClient,
    invocation_pane_id: &str,
    mode: RestartMode,
) -> FlowResult {
    let target = match resolve_target(client, invocation_pane_id)
        .map_err(|error| FlowError::titled(FAILURE_TITLE, error))?
    {
        Some(target) => target,
        None => {
            return Ok(Outcome::Notice {
                title: NOTIFICATION_TITLE.to_owned(),
                body: NO_AGENT.to_owned(),
            });
        }
    };

    // A plugin action does not move keyboard focus. When the action is invoked from the
    // yazi or term pane, focus the adjacent agent pane before its menus open.
    if target.pane_id != invocation_pane_id
        && !focus_target(client, invocation_pane_id, &target.pane_id)
            .map_err(|error| FlowError::titled(FAILURE_TITLE, error))?
    {
        return Err(FlowError::titled(NOTIFICATION_TITLE, anyhow!(CANNOT_FOCUS)).into());
    }

    let fallback = restart_resolved(client, &target, mode)
        .map_err(|error| FlowError::titled(FAILURE_TITLE, error))?;
    Ok(match fallback {
        Some(body) => Outcome::Notice {
            title: NOTIFICATION_TITLE.to_owned(),
            body: body.to_owned(),
        },
        None => Outcome::Done,
    })
}

fn resolve_target(client: &dyn HerdrClient, invocation_pane_id: &str) -> Result<Option<Pane>> {
    let invocation = client
        .pane_get(json!({ "pane_id": invocation_pane_id }))
        .context("failed to read the invocation pane")?
        .pane;
    if invocation.agent.is_some() {
        return Ok(Some(invocation));
    }

    let panes = client
        .pane_list(json!({}))
        .context("failed to list panes in the invocation tab")?
        .panes;
    Ok(panes
        .into_iter()
        .find(|pane| pane.tab_id == invocation.tab_id && pane.agent.is_some()))
}

fn invocation_pane_id() -> Result<String> {
    let context_id = env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| value["focused_pane_id"].as_str().map(str::to_owned))
        .filter(|value| !value.is_empty());
    context_id
        .or_else(|| {
            env::var("HERDR_ACTIVE_PANE_ID")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .context("plugin context does not identify the focused pane")
}

fn focus_target(
    client: &dyn HerdrClient,
    invocation_pane_id: &str,
    target_id: &str,
) -> Result<bool> {
    for direction in ["left", "right", "up", "down"] {
        let neighbor = client
            .pane_neighbor(json!({ "pane_id": invocation_pane_id, "direction": direction }))
            .with_context(|| format!("failed to read the {direction} pane neighbour"))?;
        let neighbor_id = neighbor
            .neighbor
            .as_ref()
            .and_then(|neighbor| neighbor.neighbor_pane_id.as_deref());
        if neighbor_id == Some(target_id) {
            client
                .pane_focus_direction(
                    json!({ "pane_id": invocation_pane_id, "direction": direction }),
                )
                .with_context(|| format!("failed to focus the {direction} pane"))?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Kills the harness and reinjects the launcher. Returns the reason a requested resume
/// fell back to a fresh start, so the caller can say so instead of quietly starting over.
fn restart_resolved(
    client: &dyn HerdrClient,
    target: &Pane,
    mode: RestartMode,
) -> Result<Option<&'static str>> {
    // The worker reads the state file directly rather than asking Herdr for the layout. The
    // config is only needed to validate a record that exists, so a pane with no record still
    // restarts when the config file is broken.
    let (record, config) = match state::read_state().panes.contains_key(&target.pane_id) {
        true => {
            let config = Config::load().context("failed to load config for agent restart")?;
            let record = state::get_for_pane(&target.pane_id, &config);
            (record, Some(config))
        }
        false => (None, None),
    };
    // Read before the kill: Herdr binds the reported session to the agent it detected, so
    // asking after the harness has exited would never see it.
    let (resume, fallback) = match mode {
        RestartMode::Resume => resolve_resume(client, target, record.as_ref(), config.as_ref())?,
        RestartMode::Fresh => (None, None),
    };

    let response = client
        .pane_process_info(json!({ "pane_id": target.pane_id }))
        .context("failed to read the agent process group")?;
    if let Some(process_info) = response.process_info {
        if let (Some(group), Some(shell)) = (
            process_info.foreground_process_group_id,
            process_info.shell_pid,
        ) {
            if should_kill(group, shell) {
                signal_group(group, libc::SIGTERM)
                    .context("failed to terminate the agent process group")?;
                for _ in 0..50 {
                    if !group_alive(group) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                if group_alive(group) {
                    signal_group(group, libc::SIGKILL)
                        .context("failed to kill the agent process group")?;
                }
                // Let the shell settle back to its prompt before injecting.
                thread::sleep(Duration::from_millis(300));
            }
        }
    }

    // pane.send_input runs the launcher as a child of the pane shell. Its final exec replaces
    // only the launcher, so killing the agent group returns the pane to its surviving shell.
    let executable = env::current_exe().context("failed to resolve the workbench executable")?;
    let label = target.label.as_deref().unwrap_or_default();
    let command = injected_command(
        &executable,
        &target.pane_id,
        label,
        record.as_ref(),
        resume.as_deref(),
    )?;
    client
        .pane_send_input(json!({
            "pane_id": target.pane_id,
            "text": command,
            "keys": ["enter"],
        }))
        .context("failed to inject the restarted agent")?;
    Ok(fallback)
}

/// The session a Resume should hand the harness, and the reason it will start fresh anyway.
fn resolve_resume(
    client: &dyn HerdrClient,
    target: &Pane,
    record: Option<&state::LastAgentRecord>,
    config: Option<&Config>,
) -> Result<(Option<String>, Option<&'static str>)> {
    let kind = match (record, config) {
        (Some(record), Some(config)) => config
            .agent(&record.agent)
            .and_then(|agent| agent.kind.as_deref()),
        _ => None,
    };
    let Some(session) = resume_session(client, &target.pane_id, record, kind)? else {
        return Ok((None, Some(NO_SESSION)));
    };
    // A harness whose kind takes no resume argument would swallow the id and start over
    // without a word. Only a record names the agent, so an unpinned pane is trusted.
    let resumable = match (record, config) {
        (Some(_), Some(_)) => crate::flows::agent::kind_can_resume(kind),
        _ => true,
    };
    Ok(match resumable {
        true => (Some(session), None),
        false => (None, Some(CANNOT_RESUME)),
    })
}

/// Herdr's own report is tried first so installing its integration later improves accuracy
/// without touching either fallback.
fn resume_session(
    client: &dyn HerdrClient,
    pane_id: &str,
    record: Option<&state::LastAgentRecord>,
    kind: Option<&str>,
) -> Result<Option<String>> {
    let pane = client
        .pane_get(json!({ "pane_id": pane_id }))
        .context("failed to read the agent session")?
        .pane;
    let reported = pane
        .agent_session
        .as_ref()
        // `kind` says what `value` is, and a transcript path is not what a harness resumes.
        .filter(|session| session.kind == "id")
        .map(|session| session.value.clone())
        .or_else(|| record.and_then(|record| record.session.clone()));
    Ok(match reported {
        Some(session) => Some(session),
        None => {
            let home = env::var_os("HOME").map(PathBuf::from);
            home.and_then(|home| sweep_session(&home, &pane, record, kind))
        }
    })
}

/// The launch-time reporter gives up after a minute, but a harness writes its session file
/// only once its first turn starts, which may be hours after the pane opened.
fn sweep_session(
    home: &Path,
    pane: &Pane,
    record: Option<&state::LastAgentRecord>,
    kind: Option<&str>,
) -> Option<String> {
    let cwd = pane.cwd.as_deref()?;
    // The record's stamp is this pane's launch, so the sweep cannot pick up a session that
    // predates the agent now running in it.
    let since_ms = record?.recorded_at.saturating_mul(1_000);
    session::resolve_session(kind?, home, Path::new(cwd), since_ms).map(|session| session.id)
}

fn should_kill(group: i32, shell: i32) -> bool {
    // The caller requires both pgid and shell pid to be present. The pgid must also be non-zero,
    // because kill on pgid 0 is unsafe here, and differ from the shell pid, which means no
    // foreground process has started yet.
    group != 0 && group != shell
}

fn signal_group(group: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc::kill(-group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Reports whether any process in the group is still running.
///
/// The zsh original polls the group **leader** (`kill -0 $pgid`). Polling the whole
/// group is the stricter reading of the same intent: a harness that outlives its leader
/// still owns the TTY, and injecting on top of it would corrupt the pane.
fn group_alive(group: i32) -> bool {
    unsafe { libc::kill(-group, 0) == 0 }
}

fn injected_command(
    executable: &Path,
    pane_id: &str,
    label: &str,
    record: Option<&state::LastAgentRecord>,
    resume: Option<&str>,
) -> Result<String> {
    let executable = executable
        .to_str()
        .context("workbench executable path is not valid UTF-8")?;
    // No tab id (the tab keeps its name), the current label as the fixed usage (so the usage
    // menu is skipped), no worktree step, and no layout. `--restart` is the restart signal;
    // `--no-layout` cannot be, because a manual launch may set it too.
    let mut argv = vec![
        executable.to_owned(),
        "agent".to_owned(),
        "launch".to_owned(),
        pane_id.to_owned(),
        "--usage".to_owned(),
        label.to_owned(),
    ];
    if let Some(record) = record {
        // The pane travels with the layout: without it the relaunch would replay the
        // layout's first agent pane, which is the wrong pin for any other agent pane.
        argv.extend([
            "--layout".to_owned(),
            record.layout.clone(),
            "--pane".to_owned(),
            record.pane.clone(),
        ]);
    }
    if let Some(resume) = resume {
        argv.extend(["--resume".to_owned(), resume.to_owned()]);
    }
    argv.extend(["--no-layout".to_owned(), "--restart".to_owned()]);
    let launcher = build_command(&argv);
    Ok(format!("{TTY_RESET}{launcher}"))
}

/// The three-way restart menu.
///
/// All three rows draw every time. Deciding whether a session id exists would mean a
/// socket call, and the popup pane dies the moment this returns — so the resolution is
/// the worker's, and a Resume with nothing to resume falls back there.
fn run_confirm() -> Result<Option<RestartMode>> {
    let (cols, lines) = popup_viewport();
    let mut menu = GumMenu::new(cols, lines);
    choose_mode(&mut menu)
}

fn choose_mode(menu: &mut impl Menu) -> Result<Option<RestartMode>> {
    // Fresh leads because it is the one row that always does what it says; resume depends on
    // a session id the menu cannot check for.
    let options = [
        FRESH_OPTION.to_owned(),
        RESUME_OPTION.to_owned(),
        CANCEL_OPTION.to_owned(),
    ];
    let Some(selection) = menu.choose(MENU_TITLE, MENU_SUBTITLE, &options, MENU_HEIGHT)? else {
        return Ok(None);
    };
    let selection = strip_pad(&selection);

    // Resolved by position, like the tab layout menu. Cancel and anything matching no row
    // land in the same arm, so an unexpected answer is a cancel rather than a panic.
    Ok(
        match options.iter().position(|option| *option == selection) {
            Some(0) => Some(RestartMode::Fresh),
            Some(1) => Some(RestartMode::Resume),
            _ => None,
        },
    )
}

/// The worker's argv tail. `--pane` carries the invocation pane, never the target: the
/// worker re-resolves so it can tell "already on the agent" from "focus must move".
fn worker_argv(pane_id: &str, mode: RestartMode) -> Vec<String> {
    let mut argv = vec![
        "agent".to_owned(),
        "restart-worker".to_owned(),
        "--pane".to_owned(),
        pane_id.to_owned(),
    ];
    if mode == RestartMode::Resume {
        argv.push("--resume".to_owned());
    }
    argv
}

/// Builds a command that outlives both the popup and the process group it will kill.
pub(crate) fn detached_command(program: &Path, args: &[String]) -> Command {
    let mut command = Command::new(program);
    // Null stdio prevents a surviving worker from holding or corrupting the popup TTY.
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // setsid creates a new session and process group. This survives the popup's SIGHUP and keeps
    // the worker outside the agent group that it terminates, so the restart can finish.
    // SAFETY: setsid is async-signal-safe and runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        })
    };
    command
}

fn spawn_worker(
    executable: &Path,
    pane_id: &str,
    mode: RestartMode,
) -> Result<std::process::Child> {
    detached_command(executable, &worker_argv(pane_id, mode))
        .spawn()
        .context("failed to spawn the restart worker")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::config::TabLayout;
    use crate::flows::menu::InputIndent;
    use crate::herdr::FakeClient;

    /// Only `choose` is exercised; the restart flow draws no other menu step.
    struct FakeMenu {
        answers: std::collections::VecDeque<Option<String>>,
        options: Vec<Vec<String>>,
    }

    impl FakeMenu {
        fn new<'a>(answers: impl IntoIterator<Item = Option<&'a str>>) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
            }
        }
    }

    impl Menu for FakeMenu {
        fn choose(
            &mut self,
            _: &str,
            _: &str,
            options: &[String],
            _: u8,
        ) -> Result<Option<String>> {
            self.options.push(options.to_vec());
            Ok(self.answers.pop_front().flatten())
        }

        fn filter(&mut self, _: &str, _: &str, _: &[String], _: &str) -> Result<Option<String>> {
            Ok(None)
        }

        fn input(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: u16,
            _: InputIndent,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn pane(id: &str, tab: &str, agent: bool) -> serde_json::Value {
        json!({"pane_id": id, "tab_id": tab, "agent": agent.then(|| json!({}))})
    }

    fn labelled_pane(id: &str, tab: &str, agent: bool, label: &str) -> serde_json::Value {
        let mut value = pane(id, tab, agent);
        value["label"] = json!(label);
        value
    }

    #[test]
    fn target_is_focused_agent_or_first_agent_in_tab() {
        let focused = FakeClient::default();
        focused.queue_response("pane.get", json!({"pane": pane("p1", "t1", true)}));
        assert_eq!(
            resolve_target(&focused, "p1").unwrap().unwrap().pane_id,
            "p1"
        );

        let fallback = FakeClient::default();
        fallback.queue_response("pane.get", json!({"pane": pane("p1", "t1", false)}));
        fallback.queue_response("pane.list", json!({"panes": [pane("p1", "t1", false), pane("p2", "t1", true), pane("p3", "t1", true)]}));
        assert_eq!(
            resolve_target(&fallback, "p1").unwrap().unwrap().pane_id,
            "p2"
        );
    }

    #[test]
    fn confirm_spawns_the_worker_with_the_invocation_pane_and_calls_no_herdr_method() {
        for mode in [RestartMode::Resume, RestartMode::Fresh] {
            let client = FakeClient::default();
            let spawned = RefCell::new(Vec::<(String, RestartMode)>::new());

            confirm_and_spawn("w1:p3", &|| Ok(Some(mode)), &|pane_id, chosen| {
                spawned.borrow_mut().push((pane_id.to_owned(), chosen));
                Ok(())
            })
            .unwrap();

            // The yazi pane the action fired from, not the agent pane it will restart.
            assert_eq!(spawned.into_inner(), [("w1:p3".to_owned(), mode)]);
            assert!(client.calls.borrow().is_empty());
        }
    }

    #[test]
    fn rejecting_the_confirmation_spawns_nothing_and_exits_cleanly() {
        let spawned = RefCell::new(0);

        assert_eq!(
            confirm_and_spawn("w1:p3", &|| Ok(None), &|_, _| {
                *spawned.borrow_mut() += 1;
                Ok(())
            })
            .unwrap(),
            None
        );

        assert_eq!(spawned.into_inner(), 0);
    }

    #[test]
    fn the_menu_always_draws_fresh_resume_and_cancel() {
        let mut menu = FakeMenu::new([Some(FRESH_OPTION)]);
        assert_eq!(choose_mode(&mut menu).unwrap(), Some(RestartMode::Fresh));
        // Three rows whatever the pane holds: the popup cannot look a session id up.
        assert_eq!(menu.options, [[FRESH_OPTION, RESUME_OPTION, CANCEL_OPTION]]);

        let mut menu = FakeMenu::new([Some(RESUME_OPTION)]);
        assert_eq!(choose_mode(&mut menu).unwrap(), Some(RestartMode::Resume));

        // Cancel, escape, and anything gum returned that matches no row all read as cancel.
        let mut menu = FakeMenu::new([Some(CANCEL_OPTION), None, Some("")]);
        for _ in 0..3 {
            assert_eq!(choose_mode(&mut menu).unwrap(), None);
        }
    }

    #[test]
    fn worker_argv_carries_the_hidden_subcommand_the_pane_flag_and_the_mode() {
        assert_eq!(
            worker_argv("w1:p3", RestartMode::Fresh),
            ["agent", "restart-worker", "--pane", "w1:p3"]
        );
        assert_eq!(
            worker_argv("w1:p3", RestartMode::Resume),
            ["agent", "restart-worker", "--pane", "w1:p3", "--resume"]
        );
    }

    #[test]
    fn resume_prefers_the_pane_snapshot_session_over_the_stored_one() {
        let stored = state::LastAgentRecord {
            agent: "codex".to_owned(),
            option: None,
            layout: "agentic-coding".to_owned(),
            pane: "agent".to_owned(),
            session: Some("stored-id".to_owned()),
            recorded_at: 1,
        };

        // Herdr's own report wins, so installing the official integration later silently
        // improves accuracy without touching the fallback.
        let reported = FakeClient::default();
        let mut pane = labelled_pane("p2", "t1", true, "review");
        pane["agent_session"] = json!({
            "source": "q.workbench",
            "agent": "codex",
            "kind": "id",
            "value": "reported-id",
        });
        reported.queue_response("pane.get", json!({"pane": pane}));
        assert_eq!(
            resume_session(&reported, "p2", Some(&stored), None).unwrap(),
            Some("reported-id".to_owned())
        );

        let unreported = FakeClient::default();
        unreported.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "review")}),
        );
        assert_eq!(
            resume_session(&unreported, "p2", Some(&stored), None).unwrap(),
            Some("stored-id".to_owned())
        );

        let neither = FakeClient::default();
        neither.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "review")}),
        );
        assert_eq!(resume_session(&neither, "p2", None, None).unwrap(), None);
    }

    /// `kind` is what says whether `value` is an id or a transcript path, and a path is
    /// not something a harness can be told to resume.
    #[test]
    fn a_session_reported_as_a_path_falls_back_to_the_stored_id() {
        let stored = state::LastAgentRecord {
            agent: "codex".to_owned(),
            option: None,
            layout: "agentic-coding".to_owned(),
            pane: "agent".to_owned(),
            session: Some("stored-id".to_owned()),
            recorded_at: 1,
        };
        let mut pane = labelled_pane("p2", "t1", true, "review");
        pane["agent_session"] = json!({
            "source": "herdr",
            "agent": "codex",
            "kind": "path",
            "value": "/Users/q/.codex/sessions/rollout-abc.jsonl",
        });

        for (record, expected) in [(Some(&stored), Some("stored-id".to_owned())), (None, None)] {
            let client = FakeClient::default();
            client.queue_response("pane.get", json!({"pane": pane}));
            assert_eq!(
                resume_session(&client, "p2", record, None).unwrap(),
                expected
            );
        }
    }

    /// The reporter polls for a minute after launch, but claude writes no transcript until
    /// its first turn — so a pane prompted later has nothing stored and only this finds it.
    #[test]
    fn a_session_written_after_the_reporter_gave_up_is_swept_at_restart() {
        let home = env::temp_dir().join(format!("workbench-sweep-{}", std::process::id()));
        let directory = home.join(".claude/projects/-Users-q-Projects-demo");
        std::fs::create_dir_all(&directory).unwrap();
        let transcript = directory.join("late-session.jsonl");
        std::fs::write(&transcript, "{}").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&transcript)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(9)),
            )
            .unwrap();
        let pane: Pane =
            serde_json::from_value(json!({"pane_id": "p2", "cwd": "/Users/q/Projects/demo"}))
                .unwrap();
        let stamped = |seconds| state::LastAgentRecord {
            agent: "claude code".to_owned(),
            option: None,
            layout: "agentic-coding".to_owned(),
            pane: "agent".to_owned(),
            session: None,
            recorded_at: seconds,
        };

        let launched_before = stamped(5);
        assert_eq!(
            sweep_session(&home, &pane, Some(&launched_before), Some("claude")),
            Some("late-session".to_owned())
        );

        // A stamp later than the transcript belongs to a launch that has written nothing yet.
        let launched_after = stamped(20);
        assert_eq!(
            sweep_session(&home, &pane, Some(&launched_after), Some("claude")),
            None
        );

        // No record means no launch stamp to bound the sweep, and no kind means no harness
        // whose files could be swept.
        assert_eq!(sweep_session(&home, &pane, None, Some("claude")), None);
        assert_eq!(
            sweep_session(&home, &pane, Some(&launched_before), None),
            None
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// `resume_args` has nothing to append for these kinds, so promising the resume would
    /// start a brand-new session without a word.
    #[test]
    fn a_kind_that_cannot_resume_reports_the_fallback() {
        let config = Config::test_default();
        let target: Pane =
            serde_json::from_value(labelled_pane("p2", "t1", true, "review")).unwrap();
        let record = |agent: &str| state::LastAgentRecord {
            agent: agent.to_owned(),
            option: None,
            layout: "agentic-coding".to_owned(),
            pane: "agent".to_owned(),
            session: Some("stored-id".to_owned()),
            recorded_at: 1,
        };

        for (agent, expected) in [
            ("codex", (Some("stored-id".to_owned()), None)),
            ("opencode", (None, Some(CANNOT_RESUME))),
        ] {
            let client = FakeClient::default();
            client.queue_response(
                "pane.get",
                json!({"pane": labelled_pane("p2", "t1", true, "review")}),
            );
            assert_eq!(
                resolve_resume(&client, &target, Some(&record(agent)), Some(&config)).unwrap(),
                expected,
                "{agent}"
            );
        }
    }

    /// Herdr binds a reported session to the agent it detected, so reading it after the
    /// harness has been killed would find nothing.
    #[test]
    fn the_session_is_read_before_the_harness_is_killed() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let client = FakeClient::default();
        client.queue_response("pane.process_info", json!({"process_info": null}));
        client.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "review")}),
        );
        let target: Pane =
            serde_json::from_value(labelled_pane("p2", "t1", true, "review")).unwrap();

        restart_resolved(&client, &target, RestartMode::Resume).unwrap();

        let calls = client.calls.borrow();
        let methods: Vec<&str> = calls.iter().map(|call| call.0.as_str()).collect();
        assert_eq!(
            methods,
            ["pane.get", "pane.process_info", "pane.send_input"]
        );
    }

    #[test]
    fn a_resolved_session_reaches_the_injected_launcher() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let client = FakeClient::default();
        client.queue_response("pane.process_info", json!({"process_info": null}));
        let mut pane = labelled_pane("p2", "t1", true, "review");
        pane["agent_session"] = json!({
            "source": "q.workbench",
            "agent": "codex",
            "kind": "id",
            "value": "reported-id",
        });
        client.queue_response("pane.get", json!({"pane": pane}));
        let target: Pane =
            serde_json::from_value(labelled_pane("p2", "t1", true, "review")).unwrap();

        assert_eq!(
            restart_resolved(&client, &target, RestartMode::Resume).unwrap(),
            None
        );

        let calls = client.calls.borrow();
        let text = calls.last().unwrap().1["text"].as_str().unwrap();
        assert!(text.contains("'--resume' 'reported-id'"), "{text}");
    }

    #[test]
    fn resume_without_any_session_id_restarts_fresh_and_says_why() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let client = FakeClient::default();
        client.queue_response("pane.process_info", json!({"process_info": null}));
        client.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "review")}),
        );
        let target: Pane =
            serde_json::from_value(labelled_pane("p2", "t1", true, "review")).unwrap();

        assert_eq!(
            restart_resolved(&client, &target, RestartMode::Resume).unwrap(),
            Some(NO_SESSION)
        );

        let calls = client.calls.borrow();
        let text = calls.last().unwrap().1["text"].as_str().unwrap();
        assert!(!text.contains("'--resume'"), "{text}");
    }

    #[test]
    fn a_fresh_restart_never_reads_the_pane_snapshot() {
        let client = FakeClient::default();
        client.queue_response("pane.process_info", json!({"process_info": null}));
        let target: Pane =
            serde_json::from_value(labelled_pane("p2", "t1", true, "review")).unwrap();

        restart_resolved(&client, &target, RestartMode::Fresh).unwrap();

        let calls = client.calls.borrow();
        let methods: Vec<&str> = calls.iter().map(|call| call.0.as_str()).collect();
        assert_eq!(methods, ["pane.process_info", "pane.send_input"]);
    }

    #[test]
    fn worker_invoked_from_a_side_pane_focuses_the_agent_before_injecting() {
        let client = FakeClient::default();
        // p3 is the term pane the action fired from; p2 is the agent pane in the same tab.
        client.queue_response("pane.get", json!({"pane": pane("p3", "t1", false)}));
        client.queue_response(
            "pane.list",
            json!({"panes": [pane("p3", "t1", false), labelled_pane("p2", "t1", true, "review")]}),
        );
        client.queue_response(
            "pane.neighbor",
            json!({"neighbor": {"neighbor_pane_id": null}}),
        );
        client.queue_response(
            "pane.neighbor",
            json!({"neighbor": {"neighbor_pane_id": "p2"}}),
        );
        client.queue_response("pane.focus_direction", json!({"type": "ok"}));
        client.queue_response("pane.process_info", json!({"process_info": null}));

        restart_worker(&client, "p3", RestartMode::Fresh).unwrap();

        let calls = client.calls.borrow();
        let methods: Vec<&str> = calls.iter().map(|call| call.0.as_str()).collect();
        assert_eq!(
            methods,
            [
                "pane.get",
                "pane.list",
                "pane.neighbor",
                "pane.neighbor",
                "pane.focus_direction",
                "pane.process_info",
                "pane.send_input",
            ]
        );
        assert_eq!(
            calls[4].1,
            json!({"pane_id": "p3", "direction": "right"}),
            "focus must move from the invocation pane towards the agent pane"
        );
        let injected = &calls[6].1;
        assert_eq!(injected["pane_id"], "p2");
        assert_eq!(injected["keys"], json!(["enter"]));
        let text = injected["text"].as_str().unwrap();
        assert!(text.starts_with(TTY_RESET));
        assert!(text.contains("'--usage' 'review'"));
    }

    #[test]
    fn missing_target_and_direction_preserve_messages_and_outcomes() {
        let missing = FakeClient::default();
        missing.queue_response("pane.get", json!({"pane": pane("p1", "t1", false)}));
        missing.queue_response("pane.list", json!({"panes": [pane("p1", "t1", false)]}));
        assert_eq!(
            restart_worker(&missing, "p1", RestartMode::Fresh).unwrap(),
            Outcome::Notice {
                title: NOTIFICATION_TITLE.to_owned(),
                body: NO_AGENT.to_owned(),
            }
        );
        assert!(!missing
            .calls
            .borrow()
            .iter()
            .any(|call| call.0 == "notification.show"));

        let blocked = FakeClient::default();
        blocked.queue_response("pane.get", json!({"pane": pane("p1", "t1", false)}));
        blocked.queue_response("pane.list", json!({"panes": [pane("p2", "t1", true)]}));
        for _ in 0..4 {
            blocked.queue_response(
                "pane.neighbor",
                json!({"neighbor": {"neighbor_pane_id": null}}),
            );
        }
        let error = restart_worker(&blocked, "p1", RestartMode::Fresh).unwrap_err();
        let flow_error = error.downcast_ref::<FlowError>().unwrap();
        assert_eq!(flow_error.title(), Some(NOTIFICATION_TITLE));
        assert_eq!(flow_error.chain(), CANNOT_FOCUS);
        assert!(!blocked
            .calls
            .borrow()
            .iter()
            .any(|call| call.0 == "notification.show"));
    }

    /// The worker is the only half that can resolve a session, so it is also the only half
    /// that can tell the user its Resume turned into a fresh start.
    #[test]
    fn a_resume_the_worker_cannot_satisfy_reaches_the_user_as_a_notice() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let client = FakeClient::default();
        for _ in 0..2 {
            client.queue_response(
                "pane.get",
                json!({"pane": labelled_pane("p2", "t1", true, "debug")}),
            );
        }
        client.queue_response("pane.process_info", json!({"process_info": null}));

        assert_eq!(
            restart_worker(&client, "p2", RestartMode::Resume).unwrap(),
            Outcome::Notice {
                title: NOTIFICATION_TITLE.to_owned(),
                body: NO_SESSION.to_owned(),
            }
        );
    }

    #[test]
    fn worker_on_the_agent_pane_itself_skips_the_focus_walk() {
        let client = FakeClient::default();
        client.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "debug")}),
        );
        client.queue_response("pane.process_info", json!({"process_info": null}));

        restart_worker(&client, "p2", RestartMode::Fresh).unwrap();

        let calls = client.calls.borrow();
        let methods: Vec<&str> = calls.iter().map(|call| call.0.as_str()).collect();
        assert_eq!(
            methods,
            ["pane.get", "pane.process_info", "pane.send_input"]
        );
    }

    #[test]
    fn focus_walk_uses_nested_neighbor_and_directional_focus() {
        let client = FakeClient::default();
        client.queue_response(
            "pane.neighbor",
            json!({"neighbor": {"neighbor_pane_id": "p2"}}),
        );
        client.queue_response("pane.focus_direction", json!({"type": "ok"}));

        assert!(focus_target(&client, "p1", "p2").unwrap());
        assert_eq!(
            client.calls.borrow().as_slice(),
            [
                (
                    "pane.neighbor".to_owned(),
                    json!({"pane_id": "p1", "direction": "left"}),
                ),
                (
                    "pane.focus_direction".to_owned(),
                    json!({"pane_id": "p1", "direction": "left"}),
                ),
            ]
        );
    }

    #[test]
    fn focus_walk_tries_every_direction_until_one_matches() {
        let client = FakeClient::default();
        // No neighbour left, a different pane right, none up, the target below. A
        // non-matching neighbour must not end the walk.
        client.queue_response("pane.neighbor", json!({}));
        client.queue_response(
            "pane.neighbor",
            json!({"neighbor": {"neighbor_pane_id": "p9"}}),
        );
        client.queue_response("pane.neighbor", json!({}));
        client.queue_response(
            "pane.neighbor",
            json!({"neighbor": {"neighbor_pane_id": "p2"}}),
        );
        client.queue_response("pane.focus_direction", json!({"type": "ok"}));

        assert!(focus_target(&client, "p1", "p2").unwrap());
        assert_eq!(
            client
                .calls
                .borrow()
                .iter()
                .map(|(method, params)| (
                    method.as_str(),
                    params["direction"].as_str().expect("a direction")
                ))
                .collect::<Vec<_>>(),
            [
                ("pane.neighbor", "left"),
                ("pane.neighbor", "right"),
                ("pane.neighbor", "up"),
                ("pane.neighbor", "down"),
                ("pane.focus_direction", "down"),
            ]
        );
    }

    #[test]
    fn process_info_uses_the_nested_response_shape() {
        let response: crate::herdr::types::PaneProcessInfoResponse =
            serde_json::from_value(json!({
                "type": "pane_process_info",
                "process_info": {
                    "foreground_process_group_id": 123,
                    "shell_pid": 456
                }
            }))
            .unwrap();
        let process_info = response.process_info.unwrap();

        assert_eq!(process_info.foreground_process_group_id, Some(123));
        assert_eq!(process_info.shell_pid, Some(456));
        assert!(should_kill(20, 10));
        assert!(!should_kill(0, 10));
        assert!(!should_kill(10, 10));
    }

    #[test]
    fn restart_without_a_record_omits_layout_and_quotes_launcher_arguments() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let command =
            injected_command(Path::new("/tmp/work bench"), "p 1", "review's", None, None).unwrap();
        assert!(command.starts_with(TTY_RESET));
        assert!(!command.contains("'--layout'"));
        assert_eq!(command, "stty sane; printf '\\033[<u\\033[?7h\\033[?25h\\033[0m'; '/tmp/work bench' 'agent' 'launch' 'p 1' '--usage' 'review'\\''s' '--no-layout' '--restart'");
    }

    #[test]
    fn restart_injects_the_stored_layout_after_the_tty_reset() {
        let _guard = crate::state::env_lock();
        let mut config = Config::test_default();
        let template = config.tab_layouts[0].clone();
        for name in ["personal-assistant", "side quest"] {
            config.tab_layouts.push(TabLayout {
                name: name.to_owned(),
                ..template.clone()
            });
        }
        // A second agent pane, so the stored pane name has something to distinguish.
        let reviewer = &mut config.tab_layouts[2].panes[1];
        reviewer.pane_type = crate::config::PaneType::Agent;
        reviewer.command = None;
        let path = env::temp_dir().join(format!("workbench-restart-state-{}", std::process::id()));
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);

        for (layout, pane, expected) in [
            (
                "personal-assistant",
                "agent",
                "'--layout' 'personal-assistant' '--pane' 'agent'",
            ),
            (
                "side quest",
                "files",
                "'--layout' 'side quest' '--pane' 'files'",
            ),
        ] {
            fs::write(
                &path,
                format!(
                    r#"{{"version":4,"panes":{{"p1":{{"agent":"codex","layout":"{layout}","pane":"{pane}","recorded_at":1}}}}}}"#
                ),
            )
            .unwrap();

            // Resolve through the state file exactly as `restart_resolved` does, so the
            // stored record still has to survive validation before it reaches the argv.
            let record = crate::state::get_for_pane("p1", &config).expect("stored record");
            let command = injected_command(
                Path::new("/tmp/workbench"),
                "p1",
                "review",
                Some(&record),
                None,
            )
            .unwrap();

            assert!(command.starts_with(TTY_RESET));
            assert!(
                command.contains(&format!("{expected} '--no-layout' '--restart'")),
                "{command}"
            );
        }
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }

    /// Reports whether a child process is still running, without reaping it.
    ///
    /// `kill(pid, 0)` cannot answer this: an unreaped child that already exited is a
    /// zombie, and signalling a zombie still succeeds.
    fn still_running(pid: i32) -> bool {
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) == 0 }
    }

    #[test]
    fn the_detached_worker_survives_a_sigterm_to_its_spawner_group() {
        // Stand in for the popup: a process in a process group of its own, so the test
        // can signal that group without signalling the test runner.
        let mut spawner = Command::new("sleep");
        spawner.arg("30").stdin(Stdio::null());
        // SAFETY: setpgid is async-signal-safe and runs between fork and exec.
        unsafe {
            spawner.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            })
        };
        let mut spawner = spawner.spawn().unwrap();
        let spawner_group = spawner.id() as i32;

        // A child that stays in the spawner's group, proving the SIGTERM really lands.
        let mut attached = Command::new("sleep");
        attached.arg("30").stdin(Stdio::null());
        // SAFETY: setpgid is async-signal-safe and runs between fork and exec.
        unsafe {
            attached.pre_exec(move || {
                if libc::setpgid(0, spawner_group) == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            })
        };
        let mut attached = attached.spawn().unwrap();

        // The worker itself, built by the production helper with a long-running body.
        let mut worker = detached_command(Path::new("/bin/sleep"), &["30".to_owned()])
            .spawn()
            .unwrap();
        let worker_pid = worker.id() as i32;

        assert_ne!(unsafe { libc::getsid(worker_pid) }, unsafe {
            libc::getsid(0)
        });
        assert_ne!(unsafe { libc::getpgid(worker_pid) }, spawner_group);

        unsafe { libc::kill(-spawner_group, libc::SIGTERM) };
        assert!(!attached.wait().unwrap().success());
        assert!(!spawner.wait().unwrap().success());
        assert!(
            still_running(worker_pid),
            "the worker must outlive a SIGTERM to its spawner's process group"
        );

        unsafe { libc::kill(worker_pid, libc::SIGKILL) };
        worker.wait().unwrap();
    }
}
