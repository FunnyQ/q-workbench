use anyhow::{bail, Result};
use serde_json::json;

use crate::config::Config;
use crate::herdr::HerdrClient;
use crate::notify::notify;
use crate::shell::shell_quote;

const TITLE: &str = "Dashboard Launcher";
const TAB_LABEL: &str = "\u{eacd}  Dashboard Launcher";
const PROMPT: &str = "/usage-dashboard and restart /cockpit server";

pub fn run(client: &dyn HerdrClient, config: &Config) -> Result<()> {
    run_with(client, &config.dashboard_workspace)
}

fn run_with(client: &dyn HerdrClient, workspace_label: &str) -> Result<()> {
    // Resolve the workspace every time because Herdr workspace IDs are not durable.
    let workspace = client
        .workspace_list(json!({}))?
        .workspaces
        .into_iter()
        .find(|workspace| workspace.label.as_deref() == Some(workspace_label));
    let Some(workspace) = workspace else {
        let body = format!("Workspace '{workspace_label}' was not found.");
        notify(client, TITLE, &body);
        bail!("{body}");
    };

    let created = client.tab_create(json!({
        "label": TAB_LABEL,
        "env": {"Q_NO_BANNER": "1"},
        "focus": true,
        "workspace_id": workspace.workspace_id,
    }))?;
    // Passing the prompt to `claude` starts processing immediately instead of leaving it staged.
    client.pane_send_input(json!({
        "pane_id": created.root_pane.pane_id,
        "text": format!("claude --model sonnet {}", shell_quote(PROMPT)),
        "keys": ["enter"],
    }))?;
    Ok(())
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

        run_with(&client, "personal-assistant").expect("launch dashboard");

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
    fn missing_workspace_notifies_and_does_not_create_a_tab() {
        let client = FakeClient::default();
        client.queue_response(
            "workspace.list",
            json!({"workspaces": [{"workspace_id": "w-other", "label": "other"}]}),
        );

        let error = run_with(&client, "personal-assistant").expect_err("reject missing workspace");

        assert_eq!(
            error.to_string(),
            "Workspace 'personal-assistant' was not found."
        );
        assert_eq!(
            client.calls.into_inner(),
            vec![
                ("workspace.list".to_owned(), json!({})),
                (
                    "notification.show".to_owned(),
                    json!({
                        "title": "Dashboard Launcher",
                        "body": "Workspace 'personal-assistant' was not found.",
                        "position": "bottom-right",
                        "sound": "none",
                    }),
                ),
            ]
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
