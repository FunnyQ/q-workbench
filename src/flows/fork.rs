use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::flows::agent::{
    build_launch, build_start, choose_pane_agent, create_popup_tab, kind_can_resume, model_row,
    AgentChoice, Continuation, PaneAgent,
};
use crate::flows::menu::{popup_viewport, GumMenu, Menu};
use crate::flows::restart::{invocation_pane_id, resolve_target, resume_session};
use crate::flows::{FlowResult, Outcome};
use crate::herdr::types::Pane;
use crate::herdr::HerdrClient;
use crate::state;

const NOTIFICATION_TITLE: &str = "Fork agent";
const NO_AGENT: &str = "No agent pane in this tab to fork.";
const NO_RECORD: &str = "This agent was not launched by workbench, so its harness is unknown.";
const CANNOT_FORK: &str = "This agent cannot fork a session.";
const NO_SESSION: &str = "No session to fork yet. Send the agent a prompt first.";
const FORK_SUFFIX: &str = " (fork)";

/// Opens a new tab of the source pane's layout whose agent continues a copy of its session.
pub fn fork(client: &dyn HerdrClient) -> FlowResult {
    // Config first: a broken config must be reported before the first socket call.
    let config = Config::load().context("failed to load config")?;
    let invocation = invocation_pane_id()?;
    let (cols, lines) = popup_viewport();
    fork_from(client, &config, &invocation, &mut GumMenu::new(cols, lines))
}

fn fork_from(
    client: &dyn HerdrClient,
    config: &Config,
    invocation_pane_id: &str,
    menu: &mut impl Menu,
) -> FlowResult {
    let Some(source) = resolve_target(client, invocation_pane_id)? else {
        return Ok(notice(NO_AGENT));
    };
    // Only the record names the harness, and with it the arguments that fork its session.
    let Some(record) = state::get_for_pane(&source.pane_id, config) else {
        return Ok(notice(NO_RECORD));
    };
    let kind = config
        .agent(&record.agent)
        .and_then(|agent| agent.kind.as_deref());
    if !kind_can_resume(kind) {
        return Ok(notice(CANNOT_FORK));
    }
    let Some(session) = resume_session(client, &source.pane_id, Some(&record), kind)? else {
        return Ok(notice(NO_SESSION));
    };
    let Some(choice) = choose_fork(config, &record, &source, &session, menu)? else {
        return Ok(Outcome::Cancelled);
    };
    let layout = config
        .layout(&record.layout)
        .expect("validated by get_for_pane");
    let workspace_id = Some(source.workspace_id).filter(|id| !id.is_empty());
    create_popup_tab(client, layout, &choice, workspace_id)?;
    Ok(Outcome::Done)
}

fn notice(body: &str) -> Outcome {
    Outcome::Notice {
        title: NOTIFICATION_TITLE.to_owned(),
        body: body.to_owned(),
    }
}

/// The fork keeps its harness, because a session belongs to one; model and effort stay
/// open. Every other agent pane of the layout is decided as a fresh launch would be.
fn choose_fork(
    config: &Config,
    record: &state::LastAgentRecord,
    source: &Pane,
    session: &str,
    menu: &mut impl Menu,
) -> Result<Option<AgentChoice>> {
    let agent = config
        .agent(&record.agent)
        .expect("validated by get_for_pane");
    // The model is locked to the running one: the prompt cache is keyed by model, so any
    // other would replay the whole forked history uncached. Only its effort stays open.
    let option = record.option.as_deref().and_then(|name| agent.option(name));
    let option_name = option.map(|option| option.name.clone());
    let mut requested_effort = record.effort.clone();
    if let Some(option) = option {
        let row = model_row(agent, option, record.effort.as_deref());
        if !row.efforts.is_empty() {
            let subtitle = "Fork this session. ←/→ sets effort.";
            let Some((_, effort)) = menu.choose_model(&agent.menu_label(), subtitle, &[row])?
            else {
                return Ok(None);
            };
            requested_effort = effort;
        }
    }
    let effort =
        option.and_then(|option| agent.resolve_effort(option, requested_effort.as_deref()));
    let continuation = Some(Continuation::Fork(session));
    let forked = PaneAgent {
        pane: record.pane.clone(),
        launch: build_launch(
            config,
            &agent.name,
            option_name.as_deref(),
            effort.as_deref(),
            continuation,
        )?,
        start: build_start(
            config,
            &agent.name,
            option_name.as_deref(),
            effort.as_deref(),
            continuation,
        ),
        kind: agent.kind.clone(),
        agent_name: agent.name.clone(),
        option_name,
        effort,
    };

    let layout = config
        .layout(&record.layout)
        .expect("validated by get_for_pane");
    let mut agents = Vec::new();
    for (_, pane) in layout.agent_panes() {
        if pane.name == record.pane {
            agents.push(forked.clone());
            continue;
        }
        let Some(agent) = choose_pane_agent(config, pane, true, None, menu)? else {
            return Ok(None);
        };
        agents.push(agent);
    }

    // The harness looks the session up under the directory it was started in.
    let project_dir = source
        .foreground_cwd
        .clone()
        .or_else(|| source.cwd.clone())
        .map(PathBuf::from)
        .context("the agent pane reports no working directory")?;
    Ok(Some(AgentChoice {
        label: fork_label(source.label.as_deref().unwrap_or(&agent.name)),
        project_dir,
        branch: None,
        agents,
    }))
}

/// A fork of a fork keeps one suffix rather than growing a chain of them.
fn fork_label(label: &str) -> String {
    match label.ends_with(FORK_SUFFIX) {
        true => label.to_owned(),
        false => format!("{label}{FORK_SUFFIX}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::env;
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::config::{PaneType, TabLayout};
    use crate::flows::menu::{InputIndent, ModelRow};
    use crate::herdr::FakeClient;

    /// Answers the harness menu with its first row and the model menu from a queue.
    #[derive(Default)]
    struct FakeMenu {
        models: VecDeque<Option<(usize, Option<String>)>>,
        model_rows: Vec<Vec<ModelRow>>,
        harness_titles: Vec<String>,
    }

    impl Menu for FakeMenu {
        fn choose(
            &mut self,
            title: &str,
            _: &str,
            options: &[String],
            _: u8,
        ) -> Result<Option<String>> {
            self.harness_titles.push(title.to_owned());
            Ok(options.first().cloned())
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

        fn choose_model(
            &mut self,
            _: &str,
            _: &str,
            rows: &[ModelRow],
        ) -> Result<Option<(usize, Option<String>)>> {
            self.model_rows.push(rows.to_vec());
            Ok(self.models.pop_front().flatten())
        }
    }

    fn record(agent: &str, option: Option<&str>, effort: Option<&str>) -> state::LastAgentRecord {
        state::LastAgentRecord {
            agent: agent.to_owned(),
            option: option.map(str::to_owned),
            effort: effort.map(str::to_owned),
            layout: "agentic-coding".to_owned(),
            pane: "agent".to_owned(),
            session: Some("stored-id".to_owned()),
            recorded_at: 1,
        }
    }

    fn source() -> Pane {
        serde_json::from_value(json!({
            "pane_id": "p1",
            "workspace_id": "w1",
            "cwd": "/Users/q/Projects/demo",
            "foreground_cwd": "/Users/q/Projects/demo/sub",
            "label": "\u{f4af}  review",
        }))
        .unwrap()
    }

    #[test]
    fn the_menu_offers_only_the_running_model_on_its_effort() {
        let config = Config::test_default();
        let mut menu = FakeMenu::default();
        menu.models.push_back(Some((0, Some("max".to_owned()))));

        let choice = choose_fork(
            &config,
            &record("claude code", Some("OpusPlan (Sonnet)"), Some("high")),
            &source(),
            "uuid-1",
            &mut menu,
        )
        .unwrap()
        .unwrap();

        let rows = &menu.model_rows[0];
        let labels = rows
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>();
        // A different model would start the fork with a cold prompt cache.
        assert_eq!(labels, ["OpusPlan (Sonnet)"]);
        assert_eq!(rows[0].efforts[rows[0].effort], "high");

        let forked = choice.agent(Some("agent")).unwrap();
        assert_eq!(
            forked.launch,
            [
                "claude",
                "--resume",
                "uuid-1",
                "--fork-session",
                "--model",
                "opusplan",
                "--effort",
                "max"
            ]
        );
        assert_eq!(
            forked.start.as_ref().unwrap().args[..3],
            ["--resume", "uuid-1", "--fork-session"]
        );
        assert_eq!(choice.label, "\u{f4af}  review (fork)");
        assert_eq!(
            choice.project_dir,
            PathBuf::from("/Users/q/Projects/demo/sub")
        );
    }

    /// With the model locked, a row with no effort leaves nothing to choose.
    #[test]
    fn a_model_without_efforts_forks_without_a_menu() {
        let config = Config::test_default();
        let mut menu = FakeMenu::default();

        let choice = choose_fork(
            &config,
            &record("claude code", Some("Opus"), None),
            &source(),
            "uuid-1",
            &mut menu,
        )
        .unwrap()
        .unwrap();

        assert!(menu.model_rows.is_empty());
        assert_eq!(choice.agents[0].option_name.as_deref(), Some("Opus"));
        assert!(choice.agents[0]
            .launch
            .contains(&"claude-opus-4-8".to_owned()));
    }

    #[test]
    fn escaping_the_model_menu_cancels_the_fork() {
        let config = Config::test_default();
        let mut menu = FakeMenu::default();
        menu.models.push_back(None);

        let choice = choose_fork(
            &config,
            &record("claude code", Some("OpusPlan (Sonnet)"), None),
            &source(),
            "uuid-1",
            &mut menu,
        )
        .unwrap();

        assert_eq!(choice, None);
    }

    /// Only the forked pane continues a session; the layout's other agent pane is asked
    /// about and launched fresh.
    #[test]
    fn a_side_agent_pane_forks_while_the_root_starts_fresh() {
        let mut config = Config::test_default();
        let mut layout: TabLayout = config.tab_layouts[0].clone();
        layout.name = "pair".to_owned();
        layout.panes[1].pane_type = PaneType::Agent;
        layout.panes[1].agent = Some("codex".to_owned());
        layout.panes[1].command = None;
        let side = layout.panes[1].name.clone();
        config.tab_layouts.push(layout);
        let mut stored = record("codex", None, None);
        stored.layout = "pair".to_owned();
        stored.pane = side.clone();
        let mut menu = FakeMenu::default();
        // The root's harness menu lands on claude, which then asks for its model.
        menu.models.push_back(Some((0, None)));

        let choice = choose_fork(&config, &stored, &source(), "019-abc", &mut menu)
            .unwrap()
            .unwrap();

        assert_eq!(
            choice.agent(Some(&side)).unwrap().launch,
            ["codex", "fork", "019-abc"]
        );
        let root = choice.agent(Some("agent")).unwrap();
        assert!(
            !root.launch.iter().any(|arg| arg == "019-abc"),
            "{:?}",
            root.launch
        );
        // The root runs its own harness menu; the forked pane runs none.
        assert_eq!(menu.harness_titles.len(), 1);
    }

    #[test]
    fn a_fork_of_a_fork_keeps_one_suffix() {
        assert_eq!(fork_label("review"), "review (fork)");
        assert_eq!(fork_label("review (fork)"), "review (fork)");
    }

    #[test]
    fn a_pane_the_plugin_never_launched_reports_instead_of_guessing() {
        let _guard = crate::state::env_lock();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
        let client = FakeClient::default();
        client.queue_response(
            "pane.get",
            json!({"pane": {"pane_id": "p1", "tab_id": "t1", "agent": {}}}),
        );

        assert_eq!(
            fork_from(
                &client,
                &Config::test_default(),
                "p1",
                &mut FakeMenu::default()
            )
            .unwrap(),
            notice(NO_RECORD)
        );
    }

    #[test]
    fn an_agent_with_no_session_yet_reports_instead_of_starting_fresh() {
        let _guard = crate::state::env_lock();
        let path = env::temp_dir().join(format!("workbench-fork-state-{}", std::process::id()));
        fs::write(
            &path,
            r#"{"version":4,"panes":{"p1":{"agent":"codex","layout":"agentic-coding","pane":"agent","recorded_at":1}}}"#,
        )
        .unwrap();
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        let client = FakeClient::default();
        for _ in 0..2 {
            client.queue_response(
                "pane.get",
                json!({"pane": {"pane_id": "p1", "tab_id": "t1", "agent": {}}}),
            );
        }

        let outcome = fork_from(
            &client,
            &Config::test_default(),
            "p1",
            &mut FakeMenu::default(),
        )
        .unwrap();

        env::remove_var("Q_WORKBENCH_STATE_FILE");
        fs::remove_file(path).unwrap();
        assert_eq!(outcome, notice(NO_SESSION));
        assert!(!client
            .calls
            .borrow()
            .iter()
            .any(|call| call.0 == "layout.apply"));
    }
}
