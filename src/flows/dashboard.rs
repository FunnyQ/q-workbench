use anyhow::Context;
use serde_json::json;

use crate::config::Config;
use crate::flows::{FlowError, FlowResult, Outcome};
use crate::herdr::HerdrClient;
use crate::shell::shell_quote;

const TITLE: &str = "Dashboard Launcher";
const TAB_LABEL: &str = "\u{eacd}  Dashboard Launcher";
const PROMPT: &str = "/usage-dashboard and restart /cockpit server";

pub fn run(client: &dyn HerdrClient, config: &Config) -> FlowResult {
    run_with(client, &config.dashboard_workspace)
}

fn run_with(client: &dyn HerdrClient, workspace_label: &str) -> FlowResult {
    // Resolve the workspace every time because Herdr workspace IDs are not durable.
    let workspace = client
        .workspace_list(json!({}))
        .context("dashboard: workspace.list")
        .map_err(|error| FlowError::titled(TITLE, error))?
        .workspaces
        .into_iter()
        .find(|workspace| workspace.label.as_deref() == Some(workspace_label));
    let Some(workspace) = workspace else {
        // The message already names the concrete cause, so it is the whole body and the
        // reporting path appends nothing to it.
        let body = format!("Workspace '{workspace_label}' was not found.");
        return Err(FlowError::complete(TITLE, body).into());
    };

    let created = client
        .tab_create(json!({
            "label": TAB_LABEL,
            "env": {"Q_NO_BANNER": "1"},
            "focus": true,
            "workspace_id": workspace.workspace_id,
        }))
        .context("dashboard: tab.create")
        .map_err(|error| FlowError::titled(TITLE, error))?;
    // Passing the prompt to `claude` starts processing immediately instead of leaving it staged.
    client
        .pane_send_input(json!({
            "pane_id": created.root_pane.pane_id,
            "text": format!("claude --model sonnet {}", shell_quote(PROMPT)),
            "keys": ["enter"],
        }))
        .context("dashboard: pane.send_input")
        .map_err(|error| FlowError::titled(TITLE, error))?;
    Ok(Outcome::Done)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use serde_json::json;

    use super::{run_with, PROMPT, TAB_LABEL};
    use crate::herdr::FakeClient;

    #[test]
    fn creates_focused_dashboard_tab_with_submitted_prompt() {
        let client = FakeClient::default();
        client.queue_response(
            "workspace.list",
            json!({
                "workspaces": [
                    {"workspace_id": "w-other", "label": "other"},
                    {"workspace_id": "w-dashboard", "label": "personal-assistant"}
                ]
            }),
        );
        client.queue_response(
            "tab.create",
            json!({
                "type": "tab.created",
                "root_pane": {"pane_id": "p-agent"},
                "tab": {"tab_id": "t-dashboard"}
            }),
        );

        assert_eq!(
            run_with(&client, "personal-assistant").expect("launch dashboard"),
            crate::flows::Outcome::Done
        );

        assert_eq!(
            client.calls.into_inner(),
            vec![
                ("workspace.list".to_owned(), json!({})),
                (
                    "tab.create".to_owned(),
                    json!({
                        "label": TAB_LABEL,
                        "env": {"Q_NO_BANNER": "1"},
                        "focus": true,
                        "workspace_id": "w-dashboard",
                    }),
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({
                        "pane_id": "p-agent",
                        "text": "claude --model sonnet '/usage-dashboard and restart /cockpit server'",
                        "keys": ["enter"],
                    }),
                ),
            ]
        );
    }

    #[test]
    fn missing_workspace_preserves_message_and_does_not_create_a_tab() {
        let client = FakeClient::default();
        client.queue_response(
            "workspace.list",
            json!({"workspaces": [{"workspace_id": "w-other", "label": "other"}]}),
        );

        let error = run_with(&client, "personal-assistant").expect_err("reject missing workspace");

        let flow_error = error.downcast_ref::<crate::flows::FlowError>().unwrap();
        assert_eq!(flow_error.title(), Some("Dashboard Launcher"));
        assert_eq!(
            flow_error.prefix(),
            Some("Workspace 'personal-assistant' was not found.")
        );
        assert_eq!(
            flow_error.chain(),
            "Workspace 'personal-assistant' was not found."
        );
        assert_eq!(
            client.calls.into_inner(),
            vec![("workspace.list".to_owned(), json!({}))]
        );
    }

    #[test]
    fn workspace_list_failure_has_exact_reporting_metadata() {
        let client = FakeClient::default();
        client.queue_error("workspace.list", "unavailable", "socket closed");

        let error = run_with(&client, "personal-assistant").expect_err("reject list failure");
        let flow_error = error.downcast_ref::<crate::flows::FlowError>().unwrap();

        assert_eq!(flow_error.title(), Some("Dashboard Launcher"));
        assert_eq!(flow_error.prefix(), None);
        assert_eq!(
            flow_error.chain(),
            "dashboard: workspace.list: Herdr error unavailable: socket closed"
        );
    }

    #[test]
    fn submitted_command_round_trips_to_four_arguments() {
        let command = format!(
            "set -- claude --model sonnet {}; printf '%s\\0' \"$@\"",
            crate::shell::shell_quote(PROMPT)
        );
        let output = Command::new("zsh")
            .args(["-c", &command])
            .output()
            .expect("zsh must be available");

        assert!(output.status.success());
        assert_eq!(
            output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>(),
            [
                b"claude".as_slice(),
                b"--model".as_slice(),
                b"sonnet".as_slice(),
                PROMPT.as_bytes(),
            ]
        );
    }
}
