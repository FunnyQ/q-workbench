use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

#[cfg(test)]
use crate::config::LayoutPane;
use crate::config::{render_label, Agent, Config, PaneType, TabLayout};
use crate::flows::menu::{popup_viewport, strip_pad, GumMenu, InputIndent, Menu};
use crate::flows::{invoking_pane_cwd, nonempty_env, FlowError, FlowResult, Outcome, PaneCwd};
use crate::herdr::HerdrClient;
use crate::shell::build_command;
use crate::state;

const HARNESS_TITLE: &str = "\u{f169f}  Launch Agent";
const USE_LAST_PREFIX: &str = "\u{f0709}  use last: ";
// Two spaces after the glyph. `scripts/agent-launcher.zsh:183` used one; the unified
// flow follows the popup and the parity contract (GLY-2).
const USAGE_TITLE: &str = "\u{f27b}  Usage";
const USAGE_DISCUSS: &str = "\u{f442}  discuss";
const USAGE_REVIEW: &str = "\u{f4af}  review";
const USAGE_DEBUG: &str = "\u{ead8}  debug";
// U+2026, one character — not three full stops.
const USAGE_WRITE: &str = "\u{f19b9}  let me write…";
const WORKTREE_TITLE: &str = "  New Worktree";
const WORKTREE_SUBTITLE: &str = "Filter a branch, or name a new one.";
const AGENT_LABEL: &str = "\u{f169f}  agent";
#[cfg(test)]
const TEST_CLAUDE_LABEL: &str = "\u{f15ce}  claude code";
#[cfg(test)]
const TEST_CODEX_LABEL: &str = "\u{ee0d}  codex";
#[cfg(test)]
const TEST_OPENCODE_LABEL: &str = "\u{f169f}  opencode";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
    pub no_layout: bool,
    pub restart: bool,
    pub layout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
    pub layout: Option<String>,
}

pub(crate) fn resolve_layout<'a>(
    config: &'a Config,
    requested: Option<&str>,
) -> Result<&'a TabLayout> {
    let name = requested.unwrap_or(&config.default_tab_layout);
    config
        .layout(name)
        .with_context(|| format!("unknown tab layout: {name}"))
}

/// Run every menu at full width, create side panes last, then replace this process.
pub fn launch(client: &dyn HerdrClient, config: &Config, options: &LaunchOptions) -> FlowResult {
    let layout = resolve_layout(config, options.layout.as_deref())?;
    let pane = client
        .pane_get(json!({ "pane_id": options.pane_id }))
        .context("failed to read the agent pane")?
        .pane;
    let cwd = pane
        .foreground_cwd
        .or(pane.cwd)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .map(Ok)
        .unwrap_or_else(|| {
            std::env::current_dir().context("failed to read the current directory")
        })?;
    let (cols, lines) = pane_viewport(client, &options.pane_id);
    let Some(mut choice) = choose_agent(
        config,
        layout,
        &cwd,
        options.worktree,
        options.usage.as_deref(),
        cols,
        lines,
        options
            .restart
            .then(|| state::get_for_pane(&options.pane_id, config))
            .flatten(),
    )?
    else {
        return Ok(Outcome::Cancelled);
    };

    if let (Some(repo_root), Some(branch)) = (RealGit.toplevel(&cwd), choice.branch.clone()) {
        match realise_worktree(&repo_root, &branch) {
            Some(directory) => choice.project_dir = directory,
            None => choice = without_worktree(choice, &repo_root),
        }
    }
    apply_launch_layout(client, layout, options, &choice)?;
    std::env::set_current_dir(&choice.project_dir)
        .with_context(|| format!("failed to enter {}", choice.project_dir.display()))?;
    Command::new("clear")
        .status()
        .context("failed to clear the terminal before launching the agent")?;

    let record = last_agent_record(&choice, layout)?;
    let _ = state::write_state(client, &options.pane_id, &record);

    // A child wrapper breaks restart-in-place. exec only returns when execvp fails.
    let error = Command::new(&choice.launch[0])
        .args(&choice.launch[1..])
        .exec();
    let message = format!("Could not launch the agent: {error}");
    Err(FlowError::titled("Agent launch failed", anyhow!(message)).into())
}

pub fn inject(client: &dyn HerdrClient, options: &InjectOptions) -> FlowResult {
    let config = Config::load().context("failed to load config")?;
    inject_with_config(client, &config, options)
}

pub(crate) fn inject_with_config(
    client: &dyn HerdrClient,
    config: &Config,
    options: &InjectOptions,
) -> FlowResult {
    let layout = resolve_layout(config, options.layout.as_deref())?;
    let executable =
        std::env::current_exe().context("failed to resolve the workbench executable")?;
    let executable = executable
        .to_str()
        .context("workbench executable path is not valid UTF-8")?;
    let mut argv = vec![
        executable.to_owned(),
        "agent".to_owned(),
        "launch".to_owned(),
        options.pane_id.clone(),
    ];
    if let Some(tab_id) = &options.tab_id {
        argv.extend(["--tab".to_owned(), tab_id.clone()]);
    }
    if let Some(usage) = &options.usage {
        argv.extend(["--usage".to_owned(), usage.clone()]);
    }
    if options.worktree {
        argv.push("--worktree".to_owned());
    }
    if let Some(layout) = &options.layout {
        argv.extend(["--layout".to_owned(), layout.clone()]);
    }

    // At inject time the usage menu has not run, so the layout root is the best
    // available name and the generic agent label remains the fallback.
    let root = &layout.panes[0];
    let pane_label = root
        .label
        .as_deref()
        .map(|label| render_label(root.icon.as_deref(), label))
        .unwrap_or_else(|| AGENT_LABEL.to_owned());

    client
        .pane_rename(json!({ "pane_id": options.pane_id, "label": pane_label }))
        .context("failed to rename the injected agent pane")?;
    client
        .pane_send_input(json!({
            "pane_id": options.pane_id,
            "text": build_command(&argv),
            "keys": ["enter"],
        }))
        .context("failed to inject the agent launcher")?;
    Ok(Outcome::Done)
}

fn apply_launch_layout(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    options: &LaunchOptions,
    choice: &AgentChoice,
) -> Result<()> {
    let cwd = choice.project_dir.to_string_lossy();
    client
        .pane_rename(json!({ "pane_id": options.pane_id, "label": choice.label }))
        .context("failed to rename the agent pane")?;
    if let Some(tab_id) = &options.tab_id {
        client
            .tab_rename(json!({ "tab_id": tab_id, "label": choice.label }))
            .context("failed to rename the agent tab")?;
    }
    if options.no_layout {
        return Ok(());
    }

    // Splitting earlier resizes menus and prevents the selected worktree from driving
    // every pane's cwd, so both splits remain after the decision flow.
    build_side_panes(client, layout, &options.pane_id, &cwd)?;
    Ok(())
}

fn build_side_panes(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    root_pane: &str,
    cwd: &str,
) -> Result<()> {
    let mut pane_ids = BTreeMap::from([(layout.panes[0].name.as_str(), root_pane.to_owned())]);
    let mut previous_pane_id = root_pane.to_owned();
    for pane in &layout.panes[1..] {
        let target_pane_id = pane
            .split_from
            .as_deref()
            .map(|name| pane_ids.get(name).expect("validated at load"))
            .unwrap_or(&previous_pane_id);
        let direction = match pane.direction.expect("validated at load") {
            crate::config::Direction::Right => "right",
            crate::config::Direction::Down => "down",
        };
        // Config ratios are self-describing as each new pane's share, while Herdr's
        // ratio is the original pane's share after the split. For example, `files`
        // uses 0.62 so Files takes 62%, the agent keeps 38%, and Herdr receives 38%.
        let ratio = 1.0 - pane.ratio.expect("validated at load");
        let mut params = Map::from_iter([
            ("target_pane_id".to_owned(), json!(target_pane_id)),
            ("direction".to_owned(), json!(direction)),
            ("ratio".to_owned(), json!(ratio)),
            ("cwd".to_owned(), json!(cwd)),
            ("focus".to_owned(), json!(false)),
        ]);
        if !pane.env.is_empty() {
            params.insert("env".to_owned(), json!(pane.env));
        }
        let pane_id = client
            .pane_split(Value::Object(params))
            .with_context(|| format!("failed to create pane {}", pane.name))?
            .pane
            .pane_id;
        if pane_id.is_empty() {
            return Err(anyhow!(
                "pane.split returned an empty pane id for pane {}",
                pane.name
            ));
        }
        if let Some(label) = &pane.label {
            client
                .pane_rename(json!({
                    "pane_id": pane_id,
                    "label": render_label(pane.icon.as_deref(), label),
                }))
                .with_context(|| format!("failed to rename pane {}", pane.name))?;
        }
        if pane.pane_type == PaneType::Command {
            client
                .pane_send_input(json!({
                    "pane_id": pane_id,
                    "text": pane.command.as_deref().expect("validated at load"),
                    "keys": ["enter"],
                }))
                .with_context(|| format!("failed to start command in pane {}", pane.name))?;
        }
        pane_ids.insert(pane.name.as_str(), pane_id.clone());
        previous_pane_id = pane_id;
    }
    Ok(())
}

fn pane_viewport(client: &dyn HerdrClient, pane_id: &str) -> (u16, u16) {
    let layout = client.pane_layout(json!({ "pane_id": pane_id })).ok();
    let rect = layout
        .as_ref()
        .and_then(|layout| layout.fields.get("layout"))
        .and_then(|layout| layout.get("panes"))
        .and_then(Value::as_array)
        .and_then(|panes| panes.iter().find(|pane| pane["pane_id"] == pane_id))
        .and_then(|pane| pane.get("rect"));
    let (fallback_cols, fallback_lines) = popup_viewport();
    let cols = rect
        .and_then(|rect| positive_dimension(rect.get("width")))
        .unwrap_or(fallback_cols);
    let lines = rect
        .and_then(|rect| positive_dimension(rect.get("height")))
        .unwrap_or(fallback_lines);
    (cols, lines)
}

fn positive_dimension(value: Option<&Value>) -> Option<u16> {
    value?.as_u64()?.try_into().ok().filter(|value| *value > 0)
}

/// Collect a popup decision, then create and focus its tab.
pub fn popup(
    client: &dyn HerdrClient,
    worktree: bool,
    requested_layout: Option<&str>,
) -> FlowResult {
    // Config first: adopting the invoking pane's cwd queries Herdr, and a broken config
    // must be reported before the first socket call.
    let config = Config::load().context("failed to load config")?;
    let layout = resolve_layout(&config, requested_layout)?;
    popup_with_layout(client, &config, layout, worktree)
}

/// The popup flow from the invoking pane's cwd onwards, for a layout the caller has
/// already resolved.
pub(crate) fn popup_with_layout(
    client: &dyn HerdrClient,
    config: &Config,
    layout: &TabLayout,
    worktree: bool,
) -> FlowResult {
    adopt_invoking_pane_cwd(client)?;
    let cwd = std::env::current_dir().context("failed to read popup working directory")?;
    let (cols, lines) = popup_viewport();
    let Some(mut choice) = choose_agent(config, layout, &cwd, worktree, None, cols, lines, None)?
    else {
        return Ok(Outcome::Cancelled);
    };

    if let Some(branch) = choice.branch.clone() {
        let repo_root = RealGit.toplevel(&cwd).unwrap_or_else(|| cwd.clone());
        if realise_worktree(&repo_root, &branch).is_none() {
            choice = without_worktree(choice, &repo_root);
        }
    }

    create_popup_tab(client, layout, &choice, nonempty_env("HERDR_WORKSPACE_ID"))?;
    Ok(Outcome::Done)
}

fn adopt_invoking_pane_cwd(client: &dyn HerdrClient) -> Result<()> {
    // A plugin popup starts in the plugin checkout. Adopt the invoking pane before git
    // can mistake that checkout for the project repository.
    let context_json = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok();
    let active_pane_id = nonempty_env("HERDR_ACTIVE_PANE_ID");
    let pane_cwd = invoking_pane_cwd(
        client,
        context_json.as_deref(),
        active_pane_id.as_deref(),
        PaneCwd::PaneOnly,
    );
    if let Some(cwd) = pane_cwd {
        std::env::set_current_dir(&cwd)
            .with_context(|| format!("failed to adopt invoking pane cwd {}", cwd.display()))?;
    }
    Ok(())
}

fn create_popup_tab(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    workspace_id: Option<String>,
) -> Result<()> {
    let cwd = choice.project_dir.to_string_lossy();
    let mut params = Map::from_iter([
        ("label".to_owned(), json!(choice.label)),
        ("cwd".to_owned(), json!(cwd)),
        ("focus".to_owned(), json!(false)),
    ]);
    if !layout.panes[0].env.is_empty() {
        params.insert("env".to_owned(), json!(layout.panes[0].env));
    }
    if let Some(workspace_id) = workspace_id {
        params.insert("workspace_id".to_owned(), json!(workspace_id));
    }
    let created = client
        .tab_create(Value::Object(params))
        .context("failed to create agent tab")?;
    let tab_id = created.tab.tab_id;
    let agent_pane = created.root_pane.pane_id;
    let result = if tab_id.is_empty() || agent_pane.is_empty() {
        Err(anyhow!("tab.create returned an empty tab or pane id"))
    } else {
        build_popup_tab(client, layout, choice, &tab_id, &agent_pane)
    };
    if let Err(error) = result {
        if !tab_id.is_empty() {
            let _ = client.tab_close(json!({ "tab_id": tab_id }));
        }
        return Err(FlowError::prefixed(
            "Agent tab failed",
            "The incomplete tab was closed.",
            error,
        )
        .into());
    }
    Ok(())
}

fn build_popup_tab(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    tab_id: &str,
    agent_pane: &str,
) -> Result<()> {
    let cwd = choice.project_dir.to_string_lossy();
    client
        .pane_rename(json!({ "pane_id": agent_pane, "label": choice.label }))
        .context("failed to rename agent pane")?;
    client
        .tab_rename(json!({ "tab_id": tab_id, "label": choice.label }))
        .context("failed to rename agent tab")?;
    build_side_panes(client, layout, agent_pane, &cwd)?;
    client
        .pane_send_input(json!({
            "pane_id": agent_pane,
            "text": build_command(&choice.launch),
            "keys": ["enter"],
        }))
        .context("failed to start agent")?;
    client
        .tab_focus(json!({ "tab_id": tab_id }))
        .context("failed to focus agent tab")?;
    let record = last_agent_record(choice, layout)?;
    let _ = state::write_state(client, agent_pane, &record);
    Ok(())
}

fn last_agent_record(choice: &AgentChoice, layout: &TabLayout) -> Result<state::LastAgentRecord> {
    Ok(state::LastAgentRecord {
        agent: choice.agent_name.clone(),
        option: choice.option_name.clone(),
        layout: layout.name.clone(),
        recorded_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs(),
    })
}

/// One resolved launch decision.
///
/// Nothing in here exists yet. A chosen worktree is only *named*, so a caller that
/// abandons the choice leaves no directory and no branch behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChoice {
    /// The pane and tab label: the usage label, plus two spaces and the branch when a
    /// worktree was chosen.
    pub label: String,
    /// The worktree when one was chosen, else the repository toplevel or the cwd.
    pub project_dir: PathBuf,
    pub branch: Option<String>,
    /// argv, ready for `exec` or for a pane command.
    pub launch: Vec<String>,
    /// The [[agents]] entry's `name`, not its rendered label.
    pub agent_name: String,
    /// The chosen [[agents.options]] entry's `name`; None for an agent with no options.
    pub option_name: Option<String>,
}

/// Run every menu and return one decision.
///
/// This module decides; it never acts. It creates no tab, no pane, no worktree and no
/// branch, so cancelling at any menu costs nothing and needs no notification. The zsh
/// version ran `git worktree add` before the harness menu, so cancelling later left an
/// orphaned worktree directory and branch behind — deviation 6 in the parity contract.
/// Creating the chosen worktree is the caller's job, through [`realise_worktree`], once
/// a choice actually came back.
#[allow(clippy::too_many_arguments)]
pub fn choose_agent(
    config: &Config,
    layout: &TabLayout,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    cols: u16,
    lines: u16,
    last: Option<state::LastAgentRecord>,
) -> Result<Option<AgentChoice>> {
    let mut menu = GumMenu::new(cols, lines);
    choose_agent_with_last(
        config,
        layout,
        cwd,
        worktree,
        fixed_usage,
        last,
        &mut menu,
        &RealGit,
    )
}

#[cfg(test)]
fn choose_agent_with(
    config: &Config,
    layout: &TabLayout,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    menu: &mut impl Menu,
    git: &impl Git,
) -> Result<Option<AgentChoice>> {
    choose_agent_with_last(config, layout, cwd, worktree, fixed_usage, None, menu, git)
}

#[allow(clippy::too_many_arguments)]
fn choose_agent_with_last(
    config: &Config,
    layout: &TabLayout,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    last: Option<state::LastAgentRecord>,
    menu: &mut impl Menu,
    git: &impl Git,
) -> Result<Option<AgentChoice>> {
    let repo_root = git.toplevel(cwd);

    // The worktree step runs first even though creation is deferred: the chosen branch
    // names the directory every pane is born in, so it has to be known before anything
    // else. Outside a work tree the step is skipped entirely.
    let branch = match (worktree, repo_root.as_deref()) {
        (true, Some(root)) => match select_worktree(root, menu, git)? {
            Some(branch) => Some(branch),
            None => return Ok(None),
        },
        _ => None,
    };

    // A layout that pins the agent runs no harness menu, so nothing here is built for it.
    let (agent_name, stored_option) = if let Some(agent_name) = &layout.panes[0].agent {
        (agent_name.clone(), None)
    } else {
        let last = last.filter(|record| state::last_choice_is_valid(record, config));
        let use_last = last.as_ref().map(|record| {
            let label = config
                .agent(&record.agent)
                .expect("validated above")
                .menu_label();
            match &record.option {
                Some(option) => format!("{USE_LAST_PREFIX}{label} · {option}"),
                None => format!("{USE_LAST_PREFIX}{label}"),
            }
        });
        let mut harness_options = config
            .agents
            .iter()
            .map(Agent::menu_label)
            .collect::<Vec<_>>();
        if let Some(option) = &use_last {
            harness_options.insert(0, option.clone());
        }
        let Some(harness) = menu.choose(HARNESS_TITLE, "Choose a harness.", &harness_options, 8)?
        else {
            return Ok(None);
        };
        let harness = strip_pad(&harness);
        if harness.is_empty() {
            return Ok(None);
        }
        let selected_last = use_last.as_deref() == Some(harness.as_str());
        if selected_last {
            let record = last.expect("use-last entry requires a stored choice");
            (record.agent, record.option)
        } else {
            // Rendered labels are unique across agents, enforced at config load.
            let agent_name = config
                .agents
                .iter()
                .find(|agent| agent.menu_label() == harness)
                .map(|agent| agent.name.clone())
                .expect("validated at load");
            (agent_name, None)
        }
    };
    let agent = config.agent(&agent_name).expect("validated at load");
    let option_name = if let Some(option_name) =
        layout.panes[0].option_name.clone().or(stored_option)
    {
        Some(option_name)
    } else if agent.options.is_empty() {
        None
    } else {
        let options = agent
            .options
            .iter()
            .map(|option| option.name.clone())
            .collect::<Vec<_>>();
        let Some(option_name) = menu.choose(&agent.menu_label(), "Choose a model.", &options, 6)?
        else {
            return Ok(None);
        };
        let option_name = strip_pad(&option_name);
        if option_name.is_empty() {
            return Ok(None);
        }
        Some(option_name)
    };

    // A fixed usage skips the menu and is used verbatim: the restart path passes the
    // pane's current label, and the project picker passes its pinned tab label.
    let usage = match fixed_usage.or(layout.tab_label.as_deref()) {
        Some(usage) => usage.to_owned(),
        None => match select_usage(menu)? {
            Some(usage) => usage,
            None => return Ok(None),
        },
    };

    let project_dir = match (&repo_root, &branch) {
        (Some(root), Some(branch)) => worktree_path(root, branch),
        (Some(root), None) => root.clone(),
        (None, _) => cwd.to_path_buf(),
    };
    let label = compose_label(&usage, branch.as_deref());
    let launch = build_launch(config, &agent_name, option_name.as_deref())?;

    Ok(Some(AgentChoice {
        label,
        project_dir,
        branch,
        launch,
        agent_name,
        option_name,
    }))
}

fn select_usage(menu: &mut impl Menu) -> Result<Option<String>> {
    let options = [
        USAGE_DISCUSS.to_owned(),
        USAGE_REVIEW.to_owned(),
        USAGE_DEBUG.to_owned(),
        USAGE_WRITE.to_owned(),
    ];
    let Some(usage) = menu.choose(USAGE_TITLE, "What is this tab for?", &options, 8)? else {
        return Ok(None);
    };
    let usage = strip_pad(&usage);
    if usage != USAGE_WRITE {
        return Ok(if usage.is_empty() { None } else { Some(usage) });
    }

    // `--width 40` and no indent, exactly as `scripts/new-agent-popup.zsh:130` draws it.
    let Some(label) = menu.input(
        USAGE_TITLE,
        "Name this tab.",
        "label for this tab…",
        40,
        InputIndent::None,
    )?
    else {
        return Ok(None);
    };
    Ok(if label.is_empty() { None } else { Some(label) })
}

fn build_launch(
    config: &Config,
    agent_name: &str,
    option_name: Option<&str>,
) -> Result<Vec<String>> {
    let agent = config
        .agent(agent_name)
        .with_context(|| format!("no agent entry for: {agent_name}"))?;
    let option = match option_name {
        Some(option_name) => Some(
            agent
                .option(option_name)
                .with_context(|| format!("agent {agent_name} has no option: {option_name}"))?,
        ),
        None if agent.options.is_empty() => None,
        None => bail!("agent {agent_name} requires an option"),
    };

    let mut launch = option
        .and_then(|option| option.command.as_ref())
        .unwrap_or(&agent.command)
        .clone();
    launch.extend(option.into_iter().flat_map(|option| option.args.clone()));
    // A command override changes only the executable; extra args apply to every
    // launch of the agent, including overridden commands.
    launch.extend(agent.extra_args.clone());
    Ok(launch)
}

fn select_worktree(
    repo_root: &Path,
    menu: &mut impl Menu,
    git: &impl Git,
) -> Result<Option<String>> {
    git.prune_worktrees(repo_root);

    // git forbids the same branch in two worktrees, so offering a branch that is already
    // checked out would only make `git worktree add` fail later.
    let used = git.checked_out_branches(repo_root);
    let branches = git
        .branches(repo_root)
        .into_iter()
        .filter(|branch| !used.contains(branch))
        .collect::<Vec<_>>();

    // One field, two jobs: it filters the existing branches and names a new one.
    // `--width 44` is literal in the popup, not derived from the viewport.
    let selection = if branches.is_empty() {
        menu.input(
            WORKTREE_TITLE,
            WORKTREE_SUBTITLE,
            "new branch name…",
            44,
            InputIndent::Centered,
        )?
    } else {
        menu.filter(
            WORKTREE_TITLE,
            WORKTREE_SUBTITLE,
            &branches,
            "filter or name a branch…",
        )?
    };
    let Some(selection) = selection else {
        return Ok(None);
    };

    // git branch names carry no whitespace, so strip it rather than fail later.
    let branch = strip_pad(&selection)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if !branch.is_empty() {
        return Ok(Some(branch));
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    Ok(Some(format!("wt-{timestamp}")))
}

/// The git reads the worktree step needs.
///
/// Behind a trait for one reason: a test can then drive the worktree menu with no
/// repository present, which is what turns "cancelling creates nothing" into a checkable
/// claim. Creation is deliberately absent from this trait — [`realise_worktree`] is the
/// only function in this module that writes anything.
trait Git {
    fn toplevel(&self, cwd: &Path) -> Option<PathBuf>;
    fn prune_worktrees(&self, repo_root: &Path);
    fn checked_out_branches(&self, repo_root: &Path) -> BTreeSet<String>;
    fn branches(&self, repo_root: &Path) -> Vec<String>;
}

struct RealGit;

impl Git for RealGit {
    fn toplevel(&self, cwd: &Path) -> Option<PathBuf> {
        crate::registry::project::git_toplevel(cwd)
    }

    /// Drop registrations whose directory was deleted by hand. Without the prune those
    /// branches still count as checked out, so they would be hidden from the menu even
    /// though they are free.
    fn prune_worktrees(&self, repo_root: &Path) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["worktree", "prune"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn checked_out_branches(&self, repo_root: &Path) -> BTreeSet<String> {
        git_lines(repo_root, &["worktree", "list", "--porcelain"])
            .into_iter()
            .filter_map(|line| line.strip_prefix("branch refs/heads/").map(str::to_owned))
            .collect()
    }

    fn branches(&self, repo_root: &Path) -> Vec<String> {
        git_lines(
            repo_root,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )
    }
}

/// Stdout lines of a git command. A failure yields no lines, which degrades the menu to
/// the free-text field rather than aborting the flow.
fn git_lines(repo_root: &Path, args: &[&str]) -> Vec<String> {
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn worktree_path(repo_root: &Path, branch: &str) -> PathBuf {
    let parent = repo_root.parent().unwrap_or(repo_root);
    let name = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    // A slash would otherwise create a nested directory under the `-wt` sibling.
    parent
        .join(format!("{name}-wt"))
        .join(branch.replace('/', "-"))
}

/// Create or reuse the worktree for a chosen branch.
/// Returns None when `git worktree add` failed, meaning: proceed without one.
///
/// Separate from the flow on purpose: the flow only names a branch, so the caller runs
/// every menu first and calls this once, after a choice came back. That is what makes
/// cancelling free of side effects.
pub fn realise_worktree(repo_root: &Path, branch: &str) -> Option<PathBuf> {
    let directory = worktree_path(repo_root, branch);
    // Reuse a directory left over from an earlier session rather than failing on it.
    if directory.is_dir() {
        return Some(directory);
    }

    let existing_branch = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .status()
        .ok()?
        .success();
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(["worktree", "add"]);
    // `-b` only for a branch that does not exist yet; git rejects it otherwise.
    if !existing_branch {
        command.args(["-b", branch]);
    }
    command.arg(&directory);
    if existing_branch {
        command.arg(branch);
    }
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success()
        .then_some(directory)
}

/// Strip the worktree from a choice whose creation failed.
///
/// Without this a caller would split panes into a directory that was never created and
/// label the tab with a branch that does not exist. `repo_root` must be the original cwd
/// when there is no repository.
pub fn without_worktree(mut choice: AgentChoice, repo_root: &Path) -> AgentChoice {
    let branch = choice.branch.take();
    if let Some(branch) = branch {
        let suffix = format!("  {branch}");
        if let Some(usage) = choice.label.strip_suffix(&suffix) {
            choice.label = compose_label(usage, None);
        }
    }
    choice.project_dir = repo_root.to_path_buf();
    choice
}

/// The usage label, then two spaces and the branch when a worktree was chosen. The
/// suffix is what keeps parallel worktree tabs distinguishable.
fn compose_label(usage: &str, branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("{usage}  {branch}"),
        None => usage.to_owned(),
    }
}

#[cfg(test)]
mod popup {
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::herdr::FakeClient;

    static POPUP_CONFIG_ID: AtomicU64 = AtomicU64::new(0);

    fn popup_choice() -> AgentChoice {
        AgentChoice {
            label: "\u{f4af}  review".to_owned(),
            project_dir: PathBuf::from("/projects/example"),
            branch: None,
            launch: vec!["codex".to_owned(), "--profile work".to_owned()],
            agent_name: "codex".to_owned(),
            option_name: None,
        }
    }

    fn default_layout() -> TabLayout {
        let config = Config::test_default();
        config.layout(&config.default_tab_layout).unwrap().clone()
    }

    fn queue_popup_create(client: &FakeClient) {
        client.queue_response(
            "tab.create",
            json!({
                "type": "tab_created",
                "root_pane": { "pane_id": "p1" },
                "tab": { "tab_id": "t1" },
            }),
        );
    }

    fn queue_popup_splits(client: &FakeClient) {
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p2" } }));
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p3" } }));
    }

    #[test]
    fn default_layout_reproduces_side_pane_calls() {
        let client = FakeClient::default();
        queue_popup_splits(&client);

        build_side_panes(&client, &default_layout(), "root", "/projects/example").unwrap();

        let calls = client.calls.into_inner();
        // The second split carries no `env` key at all, not an empty one. The whole-vec
        // compare below covers it, but the baseline is worth naming on its own line.
        assert!(calls[3].1.get("env").is_none(), "{:?}", calls[3].1);
        assert_eq!(
            calls,
            vec![
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "root", "direction": "right", "ratio": 0.38,
                        "cwd": "/projects/example", "env": { "Q_NO_BANNER": "1" },
                        "focus": false,
                    }),
                ),
                (
                    "pane.rename".to_owned(),
                    json!({ "pane_id": "p2", "label": "\u{f0968}  Files" }),
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({ "pane_id": "p2", "text": "yazi .", "keys": ["enter"] }),
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "p2", "direction": "down", "ratio": 0.9,
                        "cwd": "/projects/example", "focus": false,
                    }),
                ),
                (
                    "pane.rename".to_owned(),
                    json!({ "pane_id": "p3", "label": "\u{f489}  term" }),
                ),
            ]
        );
    }

    #[test]
    fn split_from_branches_back_to_named_pane() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        let mut fourth = layout.panes[2].clone();
        fourth.name = "logs".to_owned();
        fourth.label = None;
        fourth.split_from = Some(layout.panes[0].name.clone());
        layout.panes.push(fourth);
        queue_popup_splits(&client);
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p4" } }));

        build_side_panes(&client, &layout, "root", "/projects/example").unwrap();

        let calls = client.calls.into_inner();
        let splits = calls
            .iter()
            .filter(|(method, _)| method == "pane.split")
            .collect::<Vec<_>>();
        assert_eq!(splits[2].1["target_pane_id"], "root");
    }

    #[test]
    fn label_less_pane_produces_no_rename() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes[1].label = None;
        queue_popup_splits(&client);

        build_side_panes(&client, &layout, "root", "/projects/example").unwrap();

        assert!(!client
            .calls
            .into_inner()
            .iter()
            .any(|(method, params)| { method == "pane.rename" && params["pane_id"] == "p2" }));
    }

    /// The plan's fifth goal: every configuration error surfaces at load, before the first
    /// socket call. `popup` used to adopt the invoking pane's cwd first, which asks Herdr
    /// for the pane, so a broken config produced a socket round trip before it was read.
    #[test]
    fn a_broken_config_stops_the_popup_before_the_first_socket_call() {
        let _guard = crate::state::env_lock();
        let directory = std::env::temp_dir().join(format!(
            "workbench-popup-config-{}-{}",
            std::process::id(),
            POPUP_CONFIG_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("create temporary directory");
        let config_file = directory.join("config.toml");
        fs::write(&config_file, "default_tab_layout = \"missing-layout\"\n").expect("write config");
        let saved = [
            "HOME",
            "Q_WORKBENCH_LOCAL_CONFIG",
            "HERDR_ACTIVE_PANE_ID",
            "HERDR_PLUGIN_CONTEXT_JSON",
        ]
        .map(|name| (name, std::env::var_os(name)));
        // Without the context JSON the cwd adoption falls through to a pane.get, which is
        // the socket call this test proves never happens.
        std::env::remove_var("HERDR_PLUGIN_CONTEXT_JSON");
        std::env::set_var("HOME", &directory);
        std::env::set_var("Q_WORKBENCH_LOCAL_CONFIG", &config_file);
        std::env::set_var("HERDR_ACTIVE_PANE_ID", "p1");

        let client = FakeClient::default();
        let error = popup(&client, false, None).expect_err("reject the broken config");

        for (name, value) in &saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        fs::remove_dir_all(&directory).expect("remove temporary directory");

        assert!(format!("{error:#}").contains("missing-layout"), "{error:#}");
        assert!(
            client.calls.borrow().is_empty(),
            "{:?}",
            client.calls.borrow()
        );
    }

    #[test]
    fn shell_pane_sends_no_input() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes.remove(1);
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p2" } }));

        build_side_panes(&client, &layout, "root", "/projects/example").unwrap();

        assert!(!client
            .calls
            .into_inner()
            .iter()
            .any(|(method, _)| method == "pane.send_input"));
    }

    #[test]
    fn empty_pane_id_fails_loudly() {
        let client = FakeClient::default();
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "" } }));

        let error =
            build_side_panes(&client, &default_layout(), "root", "/projects/example").unwrap_err();

        assert_eq!(
            error.to_string(),
            "pane.split returned an empty pane id for pane files"
        );
    }

    fn launch_options(tab_id: Option<&str>, no_layout: bool) -> LaunchOptions {
        LaunchOptions {
            pane_id: "p1".to_owned(),
            tab_id: tab_id.map(str::to_owned),
            usage: None,
            worktree: false,
            no_layout,
            restart: false,
            layout: None,
        }
    }

    #[test]
    fn resolve_layout_accepts_a_named_layout() {
        let config = Config::test_default();

        let layout = resolve_layout(&config, Some("agentic-coding")).unwrap();

        assert_eq!(layout.name, "agentic-coding");
    }

    #[test]
    fn resolve_layout_uses_the_configured_default() {
        let mut config = Config::test_default();
        config.default_tab_layout = "agentic-coding".to_owned();

        let layout = resolve_layout(&config, None).unwrap();

        assert_eq!(layout.name, config.default_tab_layout);
    }

    #[test]
    fn resolve_layout_rejects_an_unknown_name() {
        let config = Config::test_default();

        let error = resolve_layout(&config, Some("unknown-layout")).unwrap_err();

        assert!(error.to_string().contains("unknown-layout"));
    }

    #[test]
    fn launch_unknown_layout_rejects_before_socket() {
        let client = FakeClient::default();
        let config = Config::test_default();
        let mut options = launch_options(None, false);
        options.layout = Some("unknown-layout".to_owned());

        let error = launch(&client, &config, &options).unwrap_err();

        assert!(error.to_string().contains("unknown-layout"));
        assert!(client.calls.into_inner().is_empty());
    }

    #[test]
    fn launcher_builds_the_required_layout_sequence() {
        let client = FakeClient::default();
        queue_popup_splits(&client);

        let layout = default_layout();
        apply_launch_layout(
            &client,
            &layout,
            &launch_options(Some("t1"), false),
            &popup_choice(),
        )
        .unwrap();

        assert_eq!(
            client.calls.into_inner(),
            vec![
                (
                    "pane.rename".to_owned(),
                    json!({ "pane_id": "p1", "label": "\u{f4af}  review" })
                ),
                (
                    "tab.rename".to_owned(),
                    json!({ "tab_id": "t1", "label": "\u{f4af}  review" })
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "p1", "direction": "right", "ratio": 0.38,
                        "cwd": "/projects/example", "env": { "Q_NO_BANNER": "1" }, "focus": false,
                    })
                ),
                (
                    "pane.rename".to_owned(),
                    json!({
                        "pane_id": "p2",
                        "label": render_label(
                            layout.panes[1].icon.as_deref(),
                            layout.panes[1].label.as_deref().unwrap(),
                        ),
                    })
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({ "pane_id": "p2", "text": "yazi .", "keys": ["enter"] })
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "p2", "direction": "down", "ratio": 0.9,
                        "cwd": "/projects/example", "focus": false,
                    })
                ),
                (
                    "pane.rename".to_owned(),
                    json!({
                        "pane_id": "p3",
                        "label": render_label(
                            layout.panes[2].icon.as_deref(),
                            layout.panes[2].label.as_deref().unwrap(),
                        ),
                    })
                ),
            ]
        );
    }

    #[test]
    fn no_layout_skips_splits_and_tab_rename_is_optional() {
        let client = FakeClient::default();

        apply_launch_layout(
            &client,
            &default_layout(),
            &launch_options(None, true),
            &popup_choice(),
        )
        .unwrap();

        assert_eq!(
            client.calls.into_inner(),
            vec![(
                "pane.rename".to_owned(),
                json!({
                    "pane_id": "p1", "label": "\u{f4af}  review"
                })
            )]
        );
    }

    #[test]
    fn inject_renames_once_and_shell_quoting_round_trips() {
        let client = FakeClient::default();
        let options = InjectOptions {
            pane_id: "pane with ' quote".to_owned(),
            tab_id: Some("tab with space".to_owned()),
            usage: Some("review $HOME".to_owned()),
            worktree: true,
            layout: Some("my layout".to_owned()),
        };

        let mut config = Config::test_default();
        let mut layout = default_layout();
        layout.name = "my layout".to_owned();
        config.tab_layouts.push(layout);
        inject_with_config(&client, &config, &options).unwrap();

        let calls = client.calls.into_inner();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            (
                "pane.rename".to_owned(),
                json!({
                    "pane_id": "pane with ' quote", "label": AGENT_LABEL
                })
            )
        );
        assert_eq!(calls[1].0, "pane.send_input");
        assert_eq!(calls[1].1["keys"], json!(["enter"]));
        let command = calls[1].1["text"].as_str().unwrap();
        let output = Command::new("zsh")
            .args(["-c", "eval \"set -- $COMMAND\"; printf '%s\\n' \"$@\""])
            .env("COMMAND", command)
            .output()
            .unwrap();
        assert!(output.status.success());
        let argv = String::from_utf8(output.stdout).unwrap();
        let expected = [
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "agent".to_owned(),
            "launch".to_owned(),
            "pane with ' quote".to_owned(),
            "--tab".to_owned(),
            "tab with space".to_owned(),
            "--usage".to_owned(),
            "review $HOME".to_owned(),
            "--worktree".to_owned(),
            "--layout".to_owned(),
            "my layout".to_owned(),
        ];
        assert_eq!(argv.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn popup_reproduces_the_exact_ten_call_sequence() {
        let client = FakeClient::default();
        queue_popup_create(&client);
        queue_popup_splits(&client);

        let layout = default_layout();
        create_popup_tab(&client, &layout, &popup_choice(), None).unwrap();

        assert_eq!(
            client.calls.into_inner(),
            vec![
                (
                    "tab.create".to_owned(),
                    json!({
                        "label": "\u{f4af}  review",
                        "cwd": "/projects/example",
                        "env": { "Q_NO_BANNER": "1" },
                        "focus": false,
                    }),
                ),
                (
                    "pane.rename".to_owned(),
                    json!({ "pane_id": "p1", "label": "\u{f4af}  review" })
                ),
                (
                    "tab.rename".to_owned(),
                    json!({ "tab_id": "t1", "label": "\u{f4af}  review" })
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "p1", "direction": "right", "ratio": 0.38,
                        "cwd": "/projects/example", "env": { "Q_NO_BANNER": "1" }, "focus": false,
                    })
                ),
                (
                    "pane.rename".to_owned(),
                    json!({
                        "pane_id": "p2",
                        "label": render_label(
                            layout.panes[1].icon.as_deref(),
                            layout.panes[1].label.as_deref().unwrap(),
                        ),
                    })
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({ "pane_id": "p2", "text": "yazi .", "keys": ["enter"] })
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "p2", "direction": "down", "ratio": 0.9,
                        "cwd": "/projects/example", "focus": false,
                    })
                ),
                (
                    "pane.rename".to_owned(),
                    json!({
                        "pane_id": "p3",
                        "label": render_label(
                            layout.panes[2].icon.as_deref(),
                            layout.panes[2].label.as_deref().unwrap(),
                        ),
                    })
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({
                        "pane_id": "p1", "text": "'codex' '--profile work'", "keys": ["enter"],
                    })
                ),
                ("tab.focus".to_owned(), json!({ "tab_id": "t1" })),
            ]
        );
    }

    #[test]
    fn popup_workspace_id_is_omitted_when_empty_and_sent_when_present() {
        for (workspace, expected) in [(None, None), (Some("w1".to_owned()), Some(json!("w1")))] {
            let client = FakeClient::default();
            queue_popup_create(&client);
            queue_popup_splits(&client);
            create_popup_tab(&client, &default_layout(), &popup_choice(), workspace).unwrap();
            assert_eq!(
                client.calls.borrow()[0].1.get("workspace_id"),
                expected.as_ref()
            );
        }
    }

    #[test]
    fn popup_cwd_prefers_plugin_context_and_falls_back_to_active_pane() {
        let fixture = RepoFixture::new("popup-cwd");
        let context_dir = fixture.directory.join("context");
        let pane_dir = fixture.directory.join("pane");
        fs::create_dir_all(&context_dir).unwrap();
        fs::create_dir_all(&pane_dir).unwrap();
        let client = FakeClient::default();
        client.queue_response(
            "pane.get",
            json!({ "pane": { "pane_id": "p1", "cwd": pane_dir } }),
        );

        let context = json!({ "focused_pane_cwd": context_dir }).to_string();
        assert_eq!(
            invoking_pane_cwd(&client, Some(&context), Some("p1"), PaneCwd::PaneOnly),
            Some(context_dir)
        );
        assert!(client.calls.borrow().is_empty());
        assert_eq!(
            invoking_pane_cwd(&client, None, Some("p1"), PaneCwd::PaneOnly),
            Some(pane_dir)
        );
        assert_eq!(
            client.calls.into_inner(),
            [("pane.get".to_owned(), json!({ "pane_id": "p1" }))]
        );
    }

    #[test]
    fn popup_failure_at_every_post_create_step_closes_and_returns_metadata() {
        let methods = [
            "pane.rename",
            "tab.rename",
            "pane.split",
            "pane.rename",
            "pane.send_input",
            "pane.split",
            "pane.rename",
            "pane.send_input",
            "tab.focus",
        ];
        for failure_index in 0..methods.len() {
            let client = FakeClient::default();
            queue_popup_create(&client);
            let mut method_counts = std::collections::HashMap::<&str, usize>::new();
            for (index, method) in methods.iter().enumerate() {
                let count = method_counts.entry(method).or_default();
                if index == failure_index {
                    client.queue_error(method, "injected", "failure");
                    break;
                }
                if *method == "pane.split" {
                    let pane_id = if *count == 0 { "p2" } else { "p3" };
                    client.queue_response(method, json!({ "pane": { "pane_id": pane_id } }));
                } else {
                    client.queue_response(method, json!({ "type": "ok" }));
                }
                *count += 1;
            }

            let error =
                create_popup_tab(&client, &default_layout(), &popup_choice(), None).unwrap_err();
            let flow_error = error.downcast_ref::<FlowError>().unwrap();
            assert_eq!(flow_error.title(), Some("Agent tab failed"));
            assert_eq!(flow_error.prefix(), Some("The incomplete tab was closed."));
            assert!(flow_error.chain().contains("injected"));
            let calls = client.calls.borrow();
            assert_eq!(
                calls[calls.len() - 1],
                ("tab.close".to_owned(), json!({ "tab_id": "t1" }))
            );
            assert!(!calls.iter().any(|call| call.0 == "notification.show"));
        }
    }

    #[test]
    fn popup_cancelled_choice_makes_zero_calls() {
        let client = FakeClient::default();
        let choice: Option<AgentChoice> = None;
        if let Some(choice) = choice {
            create_popup_tab(&client, &default_layout(), &choice, None).unwrap();
        }
        assert!(client.calls.into_inner().is_empty());
    }

    #[test]
    fn popup_extra_args_preserve_toml_array_boundaries_and_bypass_is_opt_in() {
        let mut config = config();
        config.agents[1].extra_args = Vec::new();
        assert_eq!(build_launch(&config, "codex", None).unwrap(), ["codex"]);
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(
            build_launch(&config, "codex", None).unwrap(),
            ["codex", "--dangerously-bypass-approvals-and-sandbox"]
        );
        config.agents[1].extra_args = ["--search", "--profile", "work"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            build_launch(&config, "codex", None).unwrap(),
            ["codex", "--search", "--profile", "work"]
        );
        config.agents[1].extra_args = vec!["--profile work".to_owned()];
        assert_eq!(
            build_launch(&config, "codex", None).unwrap(),
            ["codex", "--profile work"]
        );
    }

    /// The shipped defaults plus the two `extra_args` values these tests pin, so a change
    /// to `default_agents()` cannot silently diverge from a hand-written parallel copy.
    fn config() -> Config {
        let mut config = Config::test_default();
        config.tab_layouts = Vec::new();
        config.default_tab_layout = String::new();
        config.agents[0].extra_args = vec!["argument with space".to_owned()];
        config.agents[1].extra_args = vec!["--search".to_owned()];
        config
    }

    /// A layout whose only pane is a bare agent root: nothing pinned, so every menu runs.
    fn bare_layout(name: &str) -> TabLayout {
        TabLayout {
            name: name.to_owned(),
            label: None,
            icon: None,
            tab_label: None,
            panes: vec![LayoutPane {
                name: "agent".to_owned(),
                label: None,
                icon: None,
                pane_type: PaneType::Agent,
                agent: None,
                option_name: None,
                command: None,
                direction: None,
                ratio: None,
                split_from: None,
                env: BTreeMap::new(),
            }],
        }
    }

    /// Replays scripted answers in menu order, so a test can cancel at an exact step.
    struct FakeMenu {
        answers: VecDeque<Option<String>>,
        options: Vec<Vec<String>>,
        titles: Vec<String>,
    }

    impl FakeMenu {
        fn new<'a>(answers: impl IntoIterator<Item = Option<&'a str>>) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
                titles: Vec::new(),
            }
        }

        fn answered_everything(&self) -> bool {
            self.answers.is_empty()
        }
    }

    impl Menu for FakeMenu {
        fn choose(
            &mut self,
            title: &str,
            _: &str,
            options: &[String],
            _: u8,
        ) -> Result<Option<String>> {
            self.titles.push(title.to_owned());
            self.options.push(options.to_vec());
            Ok(self.answers.pop_front().flatten())
        }
        fn filter(&mut self, _: &str, _: &str, _: &[String], _: &str) -> Result<Option<String>> {
            Ok(self.answers.pop_front().flatten())
        }
        fn input(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: u16,
            _: InputIndent,
        ) -> Result<Option<String>> {
            Ok(self.answers.pop_front().flatten())
        }
    }

    #[test]
    fn layout_omissions_drive_three_menus_in_order() {
        let config = config();
        let layout = bare_layout("ask");
        let mut menu = FakeMenu::new([Some(TEST_CLAUDE_LABEL), Some("Opus"), Some(USAGE_DISCUSS)]);

        let choice = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap();

        assert!(choice.is_some());
        assert_eq!(menu.titles.len(), 3);
        assert_eq!(menu.titles, [HARNESS_TITLE, TEST_CLAUDE_LABEL, USAGE_TITLE]);
    }

    #[test]
    fn pinned_layout_drives_zero_menus() {
        let config = config();
        let layout = {
            let mut layout = bare_layout("pinned");
            layout.tab_label = Some("Personal Assistant".to_owned());
            layout.panes[0].agent = Some("claude code".to_owned());
            layout.panes[0].option_name = Some("Opus".to_owned());
            layout
        };
        let mut menu = FakeMenu::new([]);

        let choice = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert!(menu.titles.is_empty());
        assert_eq!(choice.agent_name, "claude code");
        assert_eq!(choice.option_name.as_deref(), Some("Opus"));
        assert_eq!(choice.label, "Personal Assistant");
    }

    #[test]
    fn empty_options_skip_the_model_menu() {
        let config = config();
        let layout = bare_layout("ask");
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), Some(USAGE_REVIEW)]);

        let choice = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.titles, [HARNESS_TITLE, USAGE_TITLE]);
        assert_eq!(choice.option_name, None);
    }

    #[test]
    fn model_menu_title_follows_the_chosen_agent() {
        let mut config = config();
        config.agents[0].label = Some("Claude Custom".to_owned());
        let rendered_label = render_label(Some("\u{f15ce}"), "Claude Custom");
        let layout = {
            let mut layout = bare_layout("ask");
            layout.tab_label = Some("Assistant".to_owned());
            layout
        };
        let mut menu = FakeMenu::new([Some(rendered_label.as_str()), Some("Opus")]);

        let choice = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap();

        assert!(choice.is_some());
        assert_eq!(menu.titles[1], rendered_label);
    }

    #[test]
    fn cancellation_at_harness_model_or_usage_is_clean() {
        let config = config();
        let cases = [
            vec![None],
            vec![Some(TEST_CLAUDE_LABEL), None],
            vec![Some(TEST_CLAUDE_LABEL), Some("Opus"), None],
        ];
        let layout = bare_layout("ask");
        for answers in cases {
            let mut menu = FakeMenu::new(answers);

            let choice = choose_agent_with(
                &config,
                &layout,
                Path::new("/project"),
                false,
                None,
                &mut menu,
                &FakeGit::nowhere(),
            )
            .unwrap();

            assert_eq!(choice, None);
        }
    }

    #[test]
    fn use_last_is_first_and_skips_model_and_usage_menus() {
        let mut config = config();
        config.tab_layouts.push(default_layout());
        let entry = format!("{USE_LAST_PREFIX}{TEST_CLAUDE_LABEL} · Opus");
        let mut menu = FakeMenu::new([Some(entry.as_str())]);

        let choice = choose_agent_with_last(
            &config,
            &default_layout(),
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Opus".to_owned()),
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options.len(), 1);
        assert_eq!(menu.options[0][0], entry);
        assert_eq!(choice.agent_name, "claude code");
        assert_eq!(choice.option_name.as_deref(), Some("Opus"));
    }

    #[test]
    fn stale_last_choice_does_not_add_a_menu_entry() {
        let config = config();
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL)]);

        let choice = choose_agent_with_last(
            &config,
            &default_layout(),
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Removed".to_owned()),
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options[0][0], TEST_CLAUDE_LABEL);
        assert_eq!(choice.agent_name, "codex");
    }

    struct FakeGit {
        toplevel: Option<PathBuf>,
        branches: Vec<String>,
        checked_out: BTreeSet<String>,
    }

    impl FakeGit {
        fn repository(root: &str) -> Self {
            Self {
                toplevel: Some(PathBuf::from(root)),
                branches: vec!["main".to_owned()],
                checked_out: ["main".to_owned()].into(),
            }
        }

        fn nowhere() -> Self {
            Self {
                toplevel: None,
                branches: Vec::new(),
                checked_out: BTreeSet::new(),
            }
        }
    }

    impl Git for FakeGit {
        fn toplevel(&self, _: &Path) -> Option<PathBuf> {
            self.toplevel.clone()
        }
        fn prune_worktrees(&self, _: &Path) {}
        fn checked_out_branches(&self, _: &Path) -> BTreeSet<String> {
            self.checked_out.clone()
        }
        fn branches(&self, _: &Path) -> Vec<String> {
            self.branches.clone()
        }
    }

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    /// A throwaway git repository with one commit, so worktree behaviour can be checked
    /// against real git rather than a stand-in.
    struct RepoFixture {
        directory: PathBuf,
    }

    impl RepoFixture {
        fn new(label: &str) -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "workbench-agent-{label}-{}-{id}",
                std::process::id()
            ));
            let repo = directory.join("example");
            fs::create_dir_all(&repo).unwrap();
            Self::git(&repo, &["init", "--quiet", "--initial-branch", "main"]);
            Self::git(&repo, &["config", "user.email", "test@example.com"]);
            Self::git(&repo, &["config", "user.name", "test"]);
            fs::write(repo.join("README.md"), "fixture\n").unwrap();
            Self::git(&repo, &["add", "README.md"]);
            Self::git(&repo, &["commit", "--quiet", "-m", "first"]);
            Self { directory }
        }

        fn git(repo: &Path, args: &[&str]) {
            let status = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }

        fn repo(&self) -> PathBuf {
            self.directory.join("example")
        }

        fn worktrees(&self) -> Vec<String> {
            git_lines(&self.repo(), &["worktree", "list", "--porcelain"])
        }

        fn branches(&self) -> Vec<String> {
            git_lines(
                &self.repo(),
                &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
            )
        }
    }

    impl Drop for RepoFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn launch_commands_match_every_harness_and_model_rule() {
        let config = Config::test_default();
        assert_eq!(build_launch(&config, "codex", None).unwrap(), ["codex"]);
        assert_eq!(
            build_launch(&config, "opencode", None).unwrap(),
            ["opencode"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("CCR")).unwrap(),
            ["ccr", "code"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus")).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("OpusPlan (Sonnet)")).unwrap(),
            ["claude", "--model", "opusplan", "--effort", "medium"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("Fable 5")).unwrap(),
            ["claude", "--model", "claude-fable-5"]
        );
    }

    #[test]
    fn extra_args_reach_plain_and_command_override_options() {
        let mut config = config();
        config.agents[0].extra_args = ["--search", "--profile", "work"]
            .map(str::to_owned)
            .to_vec();

        assert_eq!(
            build_launch(&config, "claude code", Some("Opus")).unwrap(),
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "--search",
                "--profile",
                "work"
            ]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("CCR")).unwrap(),
            ["ccr", "code", "--search", "--profile", "work"]
        );
    }

    #[test]
    fn command_override_still_takes_its_option_args() {
        let mut config = config();
        config.agents[0].extra_args.clear();
        config.agents[0].options[2].args = vec!["--flag".to_owned()];

        assert_eq!(
            build_launch(&config, "claude code", Some("CCR")).unwrap(),
            ["ccr", "code", "--flag"]
        );
    }

    #[test]
    fn a_spaced_option_argument_survives_as_one_argument() {
        let mut config = config();
        config.agents[0].extra_args.clear();
        config.agents[0].command.push("code".to_owned());
        config.agents[0].options[0].args =
            ["--cd", "/Users/q/My Projects"].map(str::to_owned).to_vec();

        assert_eq!(
            build_launch(&config, "claude code", Some("Opus")).unwrap(),
            ["claude", "code", "--cd", "/Users/q/My Projects"]
        );
    }

    #[test]
    fn missing_launch_names_are_named_errors() {
        let config = config();

        let error = build_launch(&config, "missing agent", None).unwrap_err();
        assert!(error.to_string().contains("missing agent"));
        let error = build_launch(&config, "claude code", Some("missing option")).unwrap_err();
        assert!(error.to_string().contains("missing option"));
        let agent_name = "claude code";
        let error = build_launch(&config, agent_name, None).unwrap_err();
        assert!(error.to_string().contains(agent_name));
    }

    #[test]
    fn bypass_flags_are_absent_unless_configured() {
        let mut config = config();
        config.agents[0].extra_args = Vec::new();
        config.agents[1].extra_args = Vec::new();
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus")).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(build_launch(&config, "codex", None).unwrap(), ["codex"]);

        config.agents[0].extra_args = vec!["--dangerously-skip-permissions".to_owned()];
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus")).unwrap(),
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "--dangerously-skip-permissions"
            ]
        );
        assert_eq!(
            build_launch(&config, "codex", None).unwrap(),
            ["codex", "--dangerously-bypass-approvals-and-sandbox"]
        );
    }

    #[test]
    fn labels_keep_glyphs_and_append_branches() {
        assert_eq!(strip_pad("   \u{f442}  discuss"), "\u{f442}  discuss");
        assert_eq!(
            compose_label("\u{f442}  discuss", Some("feature/menu")),
            "\u{f442}  discuss  feature/menu"
        );
        assert_eq!(
            compose_label("\u{f442}  discuss", None),
            "\u{f442}  discuss"
        );
    }

    #[test]
    fn slash_in_branch_becomes_dash_in_worktree_directory() {
        assert_eq!(
            worktree_path(Path::new("/projects/example"), "feature/menu"),
            PathBuf::from("/projects/example-wt/feature-menu")
        );
        assert_eq!(
            worktree_path(Path::new("/projects/example"), "wt-1785474235"),
            PathBuf::from("/projects/example-wt/wt-1785474235")
        );
    }

    #[test]
    fn failed_worktree_choice_normalises_to_the_no_worktree_choice() {
        let config = config();

        let mut menu = FakeMenu::new([
            Some("feature/menu"),
            Some(TEST_CODEX_LABEL),
            Some(USAGE_DISCUSS),
        ]);
        let with_worktree = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            true,
            None,
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(with_worktree.label, "\u{f442}  discuss  feature/menu");
        assert_eq!(
            with_worktree.project_dir,
            Path::new("/projects/example-wt/feature-menu")
        );

        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), Some(USAGE_DISCUSS)]);
        let never_a_worktree = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            false,
            None,
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();

        let normalised = without_worktree(with_worktree, Path::new("/projects/example"));
        assert_eq!(normalised.branch, None);
        assert_eq!(normalised.project_dir, Path::new("/projects/example"));
        assert_eq!(normalised.label, never_a_worktree.label);
        assert_eq!(normalised, never_a_worktree);
    }

    #[test]
    fn a_usage_label_ending_in_the_branch_name_survives_normalisation() {
        let choice = AgentChoice {
            label: "review menu  menu".to_owned(),
            project_dir: PathBuf::from("/projects/example-wt/menu"),
            branch: Some("menu".to_owned()),
            launch: vec!["codex".to_owned()],
            agent_name: "codex".to_owned(),
            option_name: None,
        };
        let normalised = without_worktree(choice, Path::new("/projects/example"));
        assert_eq!(normalised.label, "review menu");
    }

    #[test]
    fn the_free_text_usage_path_names_the_tab() {
        let config = config();
        let mut menu = FakeMenu::new([
            Some(TEST_OPENCODE_LABEL),
            Some(USAGE_WRITE),
            Some("ship the rewrite"),
        ]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            false,
            None,
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(choice.label, "ship the rewrite");
        assert_eq!(choice.launch, ["opencode"]);
        assert_eq!(choice.agent_name, "opencode");
        assert_eq!(choice.option_name, None);
    }

    #[test]
    fn a_fixed_usage_skips_the_usage_menu_and_is_used_verbatim() {
        let config = config();

        // Only the harness answer is scripted: a usage menu would read past the end and
        // cancel the flow.
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL)]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            false,
            Some("\u{f09d1}  main"),
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(choice.label, "\u{f09d1}  main");
        assert!(menu.answered_everything());

        let mut menu = FakeMenu::new([Some(TEST_CLAUDE_LABEL), Some("Opus")]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            false,
            Some("\u{f442}  discuss"),
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(choice.label, "\u{f442}  discuss");
        assert_eq!(choice.option_name.as_deref(), Some("Opus"));
        assert!(menu.answered_everything());
    }

    #[test]
    fn an_empty_branch_name_becomes_a_timestamped_one() {
        let config = config();
        let mut menu = FakeMenu::new([Some("   "), Some(TEST_CODEX_LABEL), Some(USAGE_DEBUG)]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/projects/example"),
            true,
            None,
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        let branch = choice.branch.unwrap();
        assert!(branch.starts_with("wt-"), "branch was {branch}");
        assert!(branch["wt-".len()..].chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn cancelling_at_each_of_the_four_menus_returns_no_choice() {
        let config = config();
        let cases: [(&str, Vec<Option<&str>>); 4] = [
            ("worktree", vec![None]),
            ("harness", vec![Some("feature/menu"), None]),
            (
                "model",
                vec![Some("feature/menu"), Some(TEST_CLAUDE_LABEL), None],
            ),
            (
                "usage",
                vec![
                    Some("feature/menu"),
                    Some(TEST_CLAUDE_LABEL),
                    Some("Opus"),
                    None,
                ],
            ),
        ];
        for (menu_name, answers) in cases {
            let mut menu = FakeMenu {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
                titles: Vec::new(),
            };
            let choice = choose_agent_with(
                &config,
                &default_layout(),
                Path::new("/projects/example"),
                true,
                None,
                &mut menu,
                &FakeGit::repository("/projects/example"),
            )
            .unwrap();
            assert_eq!(choice, None, "cancelling at the {menu_name} menu");
        }
    }

    #[test]
    fn cancelling_outside_a_repository_returns_no_choice() {
        let config = config();
        let mut menu = FakeMenu::new([None]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            Path::new("/not-a-repository"),
            true,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap();
        assert_eq!(choice, None);
    }

    #[test]
    fn cancelling_in_a_real_repository_creates_no_worktree_and_no_branch() {
        let fixture = RepoFixture::new("cancel");
        let config = config();
        let worktrees_before = fixture.worktrees();
        let branches_before = fixture.branches();
        assert_eq!(branches_before, ["main"]);

        // The fixture's only branch is checked out in the main worktree, so the menu falls
        // through to the free-text field — the first answer names a branch either way.
        let cases: [(&str, Vec<Option<&str>>); 4] = [
            ("worktree", vec![None]),
            ("harness", vec![Some("feature/menu"), None]),
            (
                "model",
                vec![Some("feature/menu"), Some(TEST_CLAUDE_LABEL), None],
            ),
            (
                "usage",
                vec![
                    Some("feature/menu"),
                    Some(TEST_CLAUDE_LABEL),
                    Some("Opus"),
                    None,
                ],
            ),
        ];
        for (menu_name, answers) in cases {
            let mut menu = FakeMenu {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
                titles: Vec::new(),
            };
            let choice = choose_agent_with(
                &config,
                &default_layout(),
                &fixture.repo(),
                true,
                None,
                &mut menu,
                &RealGit,
            )
            .unwrap();
            assert_eq!(choice, None, "cancelling at the {menu_name} menu");
            assert_eq!(
                fixture.worktrees(),
                worktrees_before,
                "cancelling at the {menu_name} menu changed the worktree list"
            );
            assert_eq!(
                fixture.branches(),
                branches_before,
                "cancelling at the {menu_name} menu created a branch"
            );
        }
        assert!(!fixture.directory.join("example-wt").exists());
    }

    #[test]
    fn a_completed_choice_still_creates_nothing_until_the_caller_realises_it() {
        let fixture = RepoFixture::new("deferred");
        let config = config();
        let mut menu = FakeMenu::new([
            Some("feature/menu"),
            Some(TEST_CODEX_LABEL),
            Some(USAGE_REVIEW),
        ]);
        let choice = choose_agent_with(
            &config,
            &default_layout(),
            &fixture.repo(),
            true,
            None,
            &mut menu,
            &RealGit,
        )
        .unwrap()
        .unwrap();

        // git reports the toplevel with symlinks resolved, so the expected directory is
        // built from that path rather than from the fixture's own.
        let repo_root = RealGit.toplevel(&fixture.repo()).unwrap();
        assert_eq!(choice.branch.as_deref(), Some("feature/menu"));
        assert_eq!(
            choice.project_dir,
            repo_root.parent().unwrap().join("example-wt/feature-menu")
        );
        assert_eq!(choice.label, "\u{f4af}  review  feature/menu");
        assert_eq!(fixture.branches(), ["main"], "the flow created a branch");
        assert!(!choice.project_dir.exists(), "the flow created a directory");

        // The caller creates it, once, after the flow returned a choice.
        let created = realise_worktree(&repo_root, "feature/menu").unwrap();
        assert_eq!(created, choice.project_dir);
        assert!(created.is_dir());
        assert_eq!(fixture.branches(), ["feature/menu", "main"]);

        // A second call reuses the directory instead of failing on it.
        assert_eq!(
            realise_worktree(&repo_root, "feature/menu").unwrap(),
            choice.project_dir
        );
    }

    #[test]
    fn a_checked_out_branch_is_not_offered() {
        let fixture = RepoFixture::new("offer");
        let repo = fixture.repo();
        RepoFixture::git(&repo, &["branch", "spare"]);
        let used = RealGit.checked_out_branches(&repo);
        assert!(used.contains("main"));
        assert!(!used.contains("spare"));

        realise_worktree(&repo, "spare").unwrap();
        let used = RealGit.checked_out_branches(&repo);
        assert!(
            used.contains("spare"),
            "a live worktree must hide its branch"
        );
    }
}
