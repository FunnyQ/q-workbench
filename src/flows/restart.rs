use std::env;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::config::Config;
use crate::flows::{FlowError, FlowResult, Outcome};
use crate::herdr::types::Pane;
use crate::herdr::HerdrClient;
use crate::shell::build_command;
use crate::state;

const NO_AGENT: &str = "No agent pane in this tab to restart.";
const CANNOT_FOCUS: &str = "Could not focus the agent pane.";
const NOTIFICATION_TITLE: &str = "Restart agent";
const FAILURE_TITLE: &str = "Agent restart failed";
// Codex leaves raw mode and Kitty CSI-u enabled, breaking line wrapping and menu arrow keys.
// The detached worker cannot access the pane TTY, so this prefix must run inside the pane.
// Keep the prefix unquoted for shell interpretation; quote only the launcher path and arguments.
const TTY_RESET: &str = "stty sane; printf '\\033[<u\\033[?7h\\033[?25h\\033[0m'; ";

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
    let spawned = confirm_and_spawn(&invocation_pane_id, &run_confirm, &|pane_id| {
        spawn_worker(&executable, pane_id).map(|_| ())
    })
    .map_err(|error| FlowError::titled(FAILURE_TITLE, error))?;
    Ok(if spawned {
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
    confirm: &dyn Fn() -> Result<bool>,
    spawn: &dyn Fn(&str) -> Result<()>,
) -> Result<bool> {
    if !confirm()? {
        return Ok(false);
    }
    spawn(invocation_pane_id)?;
    Ok(true)
}

/// Restarts the agent pane reachable from `invocation_pane_id`.
///
/// Runs in the detached worker, so `invocation_pane_id` is the pane the popup was
/// opened from and may well hold yazi or a shell rather than the agent.
pub fn restart_worker(client: &dyn HerdrClient, invocation_pane_id: &str) -> FlowResult {
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

    restart_resolved(client, &target).map_err(|error| FlowError::titled(FAILURE_TITLE, error))?;
    Ok(Outcome::Done)
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

fn restart_resolved(client: &dyn HerdrClient, target: &Pane) -> Result<()> {
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
    // The worker reads the state file directly rather than asking Herdr for the layout. The
    // config is only needed to validate a record that exists, so a pane with no record still
    // restarts when the config file is broken.
    let record = match state::read_state().panes.contains_key(&target.pane_id) {
        true => {
            let config = Config::load().context("failed to load config for agent restart")?;
            state::get_for_pane(&target.pane_id, &config)
        }
        false => None,
    };
    let command = injected_command(&executable, &target.pane_id, label, record.as_ref())?;
    client
        .pane_send_input(json!({
            "pane_id": target.pane_id,
            "text": command,
            "keys": ["enter"],
        }))
        .context("failed to inject the restarted agent")?;
    Ok(())
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
        argv.extend(["--layout".to_owned(), record.layout.clone()]);
    }
    argv.extend(["--no-layout".to_owned(), "--restart".to_owned()]);
    let launcher = build_command(&argv);
    Ok(format!("{TTY_RESET}{launcher}"))
}

fn run_confirm() -> Result<bool> {
    // `COLUMNS` used to answer here, but zsh never exports it to this process, so the
    // banner was always laid out for an 80-column pane.
    let cols = super::terminal_size().map_or(80, |(cols, _lines)| cols);
    let content_width = 44_u16.min(cols.saturating_sub(4));
    let content_margin = cols.saturating_sub(content_width + 2) / 2;
    let subtitle = Command::new("gum")
        .args([
            "style",
            "--foreground",
            "240",
            "The agent will relaunch in place.",
        ])
        .output()
        .context("failed to style the restart subtitle")?;
    if !subtitle.status.success() {
        return Err(anyhow!("gum style failed for the restart subtitle"));
    }
    let subtitle = String::from_utf8(subtitle.stdout).context("gum produced invalid UTF-8")?;
    let banner = Command::new("gum")
        .args([
            "style",
            "--border",
            "rounded",
            "--padding",
            "1 3",
            "--width",
        ])
        .arg(content_width.to_string())
        .arg("--bold")
        .arg("\u{f002a}  Current session will end")
        .arg("")
        .arg(subtitle.trim_end())
        .output()
        .context("failed to style the restart banner")?;
    if !banner.status.success() {
        return Err(anyhow!("gum style failed for the restart banner"));
    }
    let banner = String::from_utf8(banner.stdout).context("gum produced invalid UTF-8")?;
    let status = Command::new("gum")
        .args([
            "confirm",
            "--affirmative",
            "Restart",
            "--negative",
            "Cancel",
            "--selected.background",
            "214",
            "--selected.foreground",
            "235",
            "--unselected.background",
            "237",
            "--unselected.foreground",
            "223",
            "--padding",
        ])
        .arg(format!("1 {content_margin}"))
        .arg(banner.trim_end())
        .status()
        .context("failed to run the restart confirmation")?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(anyhow!("gum confirm failed with status {code:?}")),
    }
}

/// The worker's argv tail. `--pane` carries the invocation pane, never the target: the
/// worker re-resolves so it can tell "already on the agent" from "focus must move".
fn worker_argv(pane_id: &str) -> Vec<String> {
    vec![
        "agent".to_owned(),
        "restart-worker".to_owned(),
        "--pane".to_owned(),
        pane_id.to_owned(),
    ]
}

/// Builds a command that outlives both the popup and the process group it will kill.
fn detached_command(program: &Path, args: &[String]) -> Command {
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

fn spawn_worker(executable: &Path, pane_id: &str) -> Result<std::process::Child> {
    detached_command(executable, &worker_argv(pane_id))
        .spawn()
        .context("failed to spawn the restart worker")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::config::TabLayout;
    use crate::herdr::FakeClient;

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
        let client = FakeClient::default();
        let spawned = RefCell::new(Vec::<String>::new());

        confirm_and_spawn("w1:p3", &|| Ok(true), &|pane_id| {
            spawned.borrow_mut().push(pane_id.to_owned());
            Ok(())
        })
        .unwrap();

        // The yazi pane the action fired from, not the agent pane it will restart.
        assert_eq!(spawned.into_inner(), ["w1:p3"]);
        assert!(client.calls.borrow().is_empty());
    }

    #[test]
    fn rejecting_the_confirmation_spawns_nothing_and_exits_cleanly() {
        let spawned = RefCell::new(0);

        confirm_and_spawn("w1:p3", &|| Ok(false), &|_| {
            *spawned.borrow_mut() += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(spawned.into_inner(), 0);
    }

    #[test]
    fn worker_argv_carries_the_hidden_subcommand_and_the_pane_flag() {
        assert_eq!(
            worker_argv("w1:p3"),
            ["agent", "restart-worker", "--pane", "w1:p3"]
        );
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

        restart_worker(&client, "p3").unwrap();

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
            restart_worker(&missing, "p1").unwrap(),
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
        let error = restart_worker(&blocked, "p1").unwrap_err();
        let flow_error = error.downcast_ref::<FlowError>().unwrap();
        assert_eq!(flow_error.title(), Some(NOTIFICATION_TITLE));
        assert_eq!(flow_error.chain(), CANNOT_FOCUS);
        assert!(!blocked
            .calls
            .borrow()
            .iter()
            .any(|call| call.0 == "notification.show"));
    }

    #[test]
    fn worker_on_the_agent_pane_itself_skips_the_focus_walk() {
        let client = FakeClient::default();
        client.queue_response(
            "pane.get",
            json!({"pane": labelled_pane("p2", "t1", true, "debug")}),
        );
        client.queue_response("pane.process_info", json!({"process_info": null}));

        restart_worker(&client, "p2").unwrap();

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
            injected_command(Path::new("/tmp/work bench"), "p 1", "review's", None).unwrap();
        assert!(command.starts_with(TTY_RESET));
        assert!(!command.contains("'--layout'"));
        assert_eq!(command, "stty sane; printf '\\033[<u\\033[?7h\\033[?25h\\033[0m'; '/tmp/work bench' 'agent' 'launch' 'p 1' '--usage' 'review'\\''s' '--no-layout' '--restart'");
    }

    #[test]
    fn restart_injects_the_stored_layout_after_the_tty_reset() {
        let _guard = crate::state::env_lock();
        let mut config = Config::test_default();
        for name in ["personal-assistant", "side quest"] {
            config.tab_layouts.push(TabLayout {
                name: name.to_owned(),
                label: None,
                icon: None,
                tab_label: None,
                panes: Vec::new(),
            });
        }
        let path = env::temp_dir().join(format!("workbench-restart-state-{}", std::process::id()));
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);

        for (layout, expected) in [
            ("personal-assistant", "'--layout' 'personal-assistant'"),
            ("side quest", "'--layout' 'side quest'"),
        ] {
            fs::write(
                &path,
                format!(
                    r#"{{"version":2,"panes":{{"p1":{{"agent":"codex","layout":"{layout}","recorded_at":1}}}}}}"#
                ),
            )
            .unwrap();

            // Resolve through the state file exactly as `restart_resolved` does, so the
            // stored record still has to survive validation before it reaches the argv.
            let record = crate::state::get_for_pane("p1", &config).expect("stored record");
            let command =
                injected_command(Path::new("/tmp/workbench"), "p1", "review", Some(&record))
                    .unwrap();

            assert!(command.starts_with(TTY_RESET));
            assert!(command.contains(&format!("{expected} '--no-layout' '--restart'")));
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
