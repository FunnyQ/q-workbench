use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{self, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::config::Config;
#[cfg(test)]
use crate::config::{Agent, AgentOption};
use crate::flows::{invoking_pane_cwd, nonempty_env, FlowError, FlowResult, Outcome, PaneCwd};
use crate::herdr::HerdrClient;
use crate::shell::build_command;
use crate::state::{self, HARNESS_CLAUDE, HARNESS_CODEX, HARNESS_OPENCODE};

const HARNESS_TITLE: &str = "\u{f169f}  Launch Agent";
const USE_LAST_PREFIX: &str = "\u{f0709}  use last: ";
// Two spaces after the glyph. `scripts/agent-launcher.zsh:183` used one; the unified
// flow follows the popup and the parity contract (GLY-2).
const MODEL_TITLE: &str = "\u{f09d1}  claude code";
const USAGE_TITLE: &str = "\u{f27b}  Usage";
const USAGE_DISCUSS: &str = "\u{f442}  discuss";
const USAGE_REVIEW: &str = "\u{f4af}  review";
const USAGE_DEBUG: &str = "\u{ead8}  debug";
// U+2026, one character — not three full stops.
const USAGE_WRITE: &str = "\u{f19b9}  let me write…";
const WORKTREE_TITLE: &str = "  New Worktree";
const WORKTREE_SUBTITLE: &str = "Filter a branch, or name a new one.";
const FILES_LABEL: &str = "\u{f0968}  Files";
const TERM_LABEL: &str = "\u{f489}  term";
const AGENT_LABEL: &str = "\u{f169f}  agent";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
    pub no_layout: bool,
    pub restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectOptions {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub usage: Option<String>,
    pub worktree: bool,
}

/// Run every menu at full width, create side panes last, then replace this process.
pub fn launch(client: &dyn HerdrClient, config: &Config, options: &LaunchOptions) -> FlowResult {
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
    apply_launch_layout(client, options, &choice)?;
    std::env::set_current_dir(&choice.project_dir)
        .with_context(|| format!("failed to enter {}", choice.project_dir.display()))?;
    Command::new("clear")
        .status()
        .context("failed to clear the terminal before launching the agent")?;

    let _ = state::write_state(
        client,
        &options.pane_id,
        &choice.harness,
        choice.model_label.as_deref(),
    );

    // A child wrapper breaks restart-in-place. exec only returns when execvp fails.
    let error = Command::new(&choice.launch[0])
        .args(&choice.launch[1..])
        .exec();
    let message = format!("Could not launch the agent: {error}");
    Err(FlowError::titled("Agent launch failed", anyhow!(message)).into())
}

pub fn inject(client: &dyn HerdrClient, options: &InjectOptions) -> FlowResult {
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

    client
        .pane_rename(json!({ "pane_id": options.pane_id, "label": AGENT_LABEL }))
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
    build_side_panes(client, &options.pane_id, &cwd)?;
    Ok(())
}

/// The Files/term half of the three-pane layout, shared by both entry points.
///
/// The popup builds the tab first and the in-pane launcher defers the splits until
/// after its menus, but the six calls between them are identical — same ratios, same
/// labels, same `Q_NO_BANNER` on the first split only — so they are written once. A
/// drift in ratio or label between the two would be an accident, never a decision.
fn build_side_panes(client: &dyn HerdrClient, agent_pane: &str, cwd: &str) -> Result<()> {
    let files_pane = client
        .pane_split(json!({
            "target_pane_id": agent_pane,
            "direction": "right",
            "ratio": 0.38,
            "cwd": cwd,
            "env": { "Q_NO_BANNER": "1" },
            "focus": false,
        }))
        .context("failed to create files pane")?
        .pane
        .pane_id;
    if files_pane.is_empty() {
        return Err(anyhow!("first pane.split returned an empty pane id"));
    }
    client
        .pane_rename(json!({ "pane_id": files_pane, "label": FILES_LABEL }))
        .context("failed to rename files pane")?;
    client
        .pane_send_input(json!({ "pane_id": files_pane, "text": "yazi .", "keys": ["enter"] }))
        .context("failed to start yazi")?;
    let term_pane = client
        .pane_split(json!({
            "target_pane_id": files_pane,
            "direction": "down",
            "ratio": 0.9,
            "cwd": cwd,
            "focus": false,
        }))
        .context("failed to create terminal pane")?
        .pane
        .pane_id;
    if term_pane.is_empty() {
        return Err(anyhow!("second pane.split returned an empty pane id"));
    }
    client
        .pane_rename(json!({ "pane_id": term_pane, "label": TERM_LABEL }))
        .context("failed to rename terminal pane")?;
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
pub fn popup(client: &dyn HerdrClient, worktree: bool) -> FlowResult {
    adopt_invoking_pane_cwd(client)?;
    let cwd = std::env::current_dir().context("failed to read popup working directory")?;
    let config = Config::load().context("failed to load config")?;
    let (cols, lines) = popup_viewport();
    let Some(mut choice) = choose_agent(&config, &cwd, worktree, None, cols, lines, None)? else {
        return Ok(Outcome::Cancelled);
    };

    if let Some(branch) = choice.branch.clone() {
        let repo_root = RealGit.toplevel(&cwd).unwrap_or_else(|| cwd.clone());
        if realise_worktree(&repo_root, &branch).is_none() {
            choice = without_worktree(choice, &repo_root);
        }
    }

    create_popup_tab(client, &choice, nonempty_env("HERDR_WORKSPACE_ID"))?;
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

fn popup_viewport() -> (u16, u16) {
    super::terminal_size().unwrap_or_else(|| {
        (
            viewport_dimension("COLUMNS", "cols"),
            viewport_dimension("LINES", "lines"),
        )
    })
}

fn viewport_dimension(variable: &str, tput_capability: &str) -> u16 {
    nonempty_env(variable)
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0)
        .or_else(|| {
            Command::new("tput")
                .arg(tput_capability)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|value| value.trim().parse::<u16>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(80)
}

fn create_popup_tab(
    client: &dyn HerdrClient,
    choice: &AgentChoice,
    workspace_id: Option<String>,
) -> Result<()> {
    let cwd = choice.project_dir.to_string_lossy();
    let mut params = Map::from_iter([
        ("label".to_owned(), json!(choice.label)),
        ("cwd".to_owned(), json!(cwd)),
        ("env".to_owned(), json!({ "Q_NO_BANNER": "1" })),
        ("focus".to_owned(), json!(false)),
    ]);
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
        build_popup_tab(client, choice, &tab_id, &agent_pane)
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
    build_side_panes(client, agent_pane, &cwd)?;
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
    let _ = state::write_state(
        client,
        agent_pane,
        &choice.harness,
        choice.model_label.as_deref(),
    );
    Ok(())
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
    /// The pad-stripped harness menu label, not a resolved binary name.
    pub harness: String,
    /// The pad-stripped model menu label; `None` for codex and opencode.
    ///
    /// The menu label is stored rather than the resolved model value: the label is what
    /// resolves through the config maps, so a renamed menu entry must not silently keep
    /// pointing at an old model.
    pub model_label: Option<String>,
}

/// Run every menu and return one decision.
///
/// This module decides; it never acts. It creates no tab, no pane, no worktree and no
/// branch, so cancelling at any menu costs nothing and needs no notification. The zsh
/// version ran `git worktree add` before the harness menu, so cancelling later left an
/// orphaned worktree directory and branch behind — deviation 6 in the parity contract.
/// Creating the chosen worktree is the caller's job, through [`realise_worktree`], once
/// a choice actually came back.
pub fn choose_agent(
    config: &Config,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    cols: u16,
    lines: u16,
    last: Option<(String, Option<String>)>,
) -> Result<Option<AgentChoice>> {
    let mut menu = GumMenu::new(cols, lines);
    choose_agent_with_last(
        config,
        cwd,
        worktree,
        fixed_usage,
        last,
        &mut menu,
        &RealGit,
    )
}

/// One menu step. `Ok(None)` means the user cancelled, which is normal and quiet.
trait Menu {
    fn choose(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        height: u8,
    ) -> Result<Option<String>>;
    fn filter(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        placeholder: &str,
    ) -> Result<Option<String>>;
    fn input(
        &mut self,
        title: &str,
        subtitle: &str,
        placeholder: &str,
        width: u16,
        indent: InputIndent,
    ) -> Result<Option<String>>;
}

/// Where a `gum input` field starts.
///
/// The two input fields are indented differently in `scripts/new-agent-popup.zsh`: the
/// branch field is preceded by `printf '%*s' "$choice_margin"` (line 78) while the usage
/// field is not (line 130). The difference is visible on screen, so it is carried across
/// rather than tidied away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputIndent {
    Centered,
    None,
}

#[cfg(test)]
fn choose_agent_with(
    config: &Config,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    menu: &mut impl Menu,
    git: &impl Git,
) -> Result<Option<AgentChoice>> {
    choose_agent_with_last(config, cwd, worktree, fixed_usage, None, menu, git)
}

fn choose_agent_with_last(
    config: &Config,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
    last: Option<(String, Option<String>)>,
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

    let regular_harnesses = [
        HARNESS_CLAUDE.to_owned(),
        HARNESS_CODEX.to_owned(),
        HARNESS_OPENCODE.to_owned(),
    ];
    let last = last
        .filter(|(harness, model)| state::last_choice_is_valid(harness, model.as_deref(), config));
    let use_last = last.as_ref().map(|(harness, model)| {
        let name = harness
            .split_once("  ")
            .map_or(harness.as_str(), |(_, name)| name);
        match model {
            Some(model) => format!("{USE_LAST_PREFIX}{name} · {model}"),
            None => format!("{USE_LAST_PREFIX}{name}"),
        }
    });
    let mut harness_options = regular_harnesses.to_vec();
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

    // harness → model → usage. The popup already asked in this order; the in-pane
    // launcher asked harness → usage → model. Deviation 1 in the parity contract picks
    // the popup's order so both entry points can share this one flow.
    let selected_last = use_last.as_deref() == Some(harness.as_str());
    let (harness, stored_model) = if selected_last {
        last.expect("use-last entry requires a stored choice")
    } else {
        (harness, None)
    };
    let model_label = if selected_last {
        stored_model
    } else if harness.contains("claude code") {
        let options: Vec<String> = config
            .menu_agent()
            .map(|agent| agent.options.iter().map(|o| o.name.clone()).collect())
            .unwrap_or_default();
        let Some(model_label) = menu.choose(MODEL_TITLE, "Choose a model.", &options, 6)? else {
            return Ok(None);
        };
        let model_label = strip_pad(&model_label);
        if model_label.is_empty() {
            return Ok(None);
        }
        Some(model_label)
    } else {
        None
    };

    // A fixed usage skips the menu and is used verbatim: the restart path passes the
    // pane's current label, and the project picker passes its pinned tab label.
    let usage = match fixed_usage {
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
    let launch = build_launch(config, &harness, model_label.as_deref())?;

    Ok(Some(AgentChoice {
        label,
        project_dir,
        branch,
        launch,
        harness,
        model_label,
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

fn build_launch(config: &Config, harness: &str, model_label: Option<&str>) -> Result<Vec<String>> {
    if harness.contains("codex") {
        let agent = config
            .agents
            .iter()
            .find(|agent| harness.contains(&agent.name))
            .context("codex harness has no agent entry")?;
        let mut launch = agent.command.clone();
        launch.extend(agent.extra_args.clone());
        return Ok(launch);
    }
    if harness.contains("opencode") {
        let agent = config
            .agents
            .iter()
            .find(|agent| harness.contains(&agent.name))
            .context("opencode harness has no agent entry")?;
        let mut launch = agent.command.clone();
        launch.extend(agent.extra_args.clone());
        return Ok(launch);
    }

    let label = model_label.context("claude harness requires a model label")?;
    if let Some(agent) = config.menu_agent() {
        if let Some(option) = agent.options.iter().find(|o| o.name == label) {
            if let Some(command) = &option.command {
                return Ok(command.clone());
            }
            let mut launch = {
                let mut cmd = agent.command.clone();
                cmd.extend(option.args.clone());
                cmd
            };
            launch.extend(agent.extra_args.clone());
            return Ok(launch);
        }
    }

    bail!("model label has no agent option entry: {label}")
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

/// Strip the leading pad but keep the Nerd Font glyph.
///
/// Menu options carry leading spaces so `gum` renders them centered. The pad is removed
/// before the selection is used, while the glyph stays: the stripped label becomes the
/// pane and tab label, and dropping the glyph would make every tab look alike.
fn strip_pad(value: &str) -> String {
    value.trim_start().to_owned()
}

/// The width of `value` in terminal columns.
///
/// Centering pads have to agree with what `gum` draws, so this mirrors how `gum` measures
/// text. Verified with `gum style --border rounded`: `中文分支` renders 8 columns and
/// `こんにちは` 10, while the Nerd Font glyph in `\u{f15ce}  claude code` renders 1, so
/// that label measures 14. Only the East Asian wide blocks and emoji count double; the
/// private-use planes the Nerd Font glyphs live in do not.
fn display_width(value: &str) -> u16 {
    value.chars().fold(0, |total, character| {
        let width = match u32::from(character) {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        };
        total.saturating_add(width)
    })
}

/// Rows `gum filter` occupies, as both the flag value and the number vertical centering
/// has to reserve.
const FILTER_HEIGHT_ARG: &str = "12";
const FILTER_HEIGHT: u16 = 12;

struct GumMenu {
    cols: u16,
    lines: u16,
}

impl GumMenu {
    fn new(cols: u16, lines: u16) -> Self {
        Self { cols, lines }
    }

    /// The banner box width: 44, narrowed on a small viewport.
    fn content_width(&self) -> u16 {
        44.min(self.cols.saturating_sub(4))
    }

    /// The left margin that centers the banner box.
    ///
    /// The box draws two columns wider than `--width`: `gum style` counts the padding
    /// inside the width and adds one border column on each side. Measured at 80 columns,
    /// `--width 44` renders 46 and leaves 17 either side.
    fn content_margin(&self) -> u16 {
        self.cols
            .saturating_sub(self.content_width())
            .saturating_sub(2)
            / 2
    }

    /// The left margin that centers a block `width` columns wide.
    fn block_margin(&self, width: u16) -> u16 {
        self.cols.saturating_sub(width) / 2
    }

    /// The blank rows that center a block `height` rows tall.
    fn vertical_padding(&self, height: u16) -> u16 {
        self.lines.saturating_sub(height) / 2
    }

    /// Draw the centered banner: title, blank line, dim subtitle.
    ///
    /// The banner is printed line by line at a computed margin instead of being handed to
    /// another `gum` call. Wrapping an already-styled multiline banner in a second
    /// `gum style` offsets its border lines, because the outer call measures the ANSI
    /// escapes as visible width and pads each line differently — the box comes out
    /// ragged. Printing the lines here keeps the border square.
    fn render_banner(&self, title: &str, subtitle: &str, body_lines: u16) -> Result<()> {
        if io::stdout().is_terminal() {
            print!("\x1b[2J\x1b[H");
        }
        let width = self.content_width();
        let subtitle = gum_output(["style", "--foreground", "240", subtitle])?.unwrap_or_default();
        let banner = gum_output([
            "style",
            "--border",
            "rounded",
            "--padding",
            "1 3",
            "--width",
            &width.to_string(),
            "--bold",
            title,
            "",
            subtitle.trim_end(),
        ])?
        .unwrap_or_default();
        // The banner is measured rather than assumed: a narrow viewport wraps the subtitle
        // and adds rows. `+ 1` is the blank line this prints between banner and body.
        let banner_lines = u16::try_from(banner.lines().count()).unwrap_or(u16::MAX);
        let block = banner_lines.saturating_add(1).saturating_add(body_lines);
        print!("{}", "\n".repeat(self.vertical_padding(block).into()));
        let margin = usize::from(self.content_margin());
        for line in banner.lines() {
            println!("{:margin$}{line}", "");
        }
        println!();
        io::stdout().flush().context("failed to draw menu banner")
    }

    /// Indent every option by one shared margin so the block is centered and its glyphs
    /// stay in a single column. `gum choose` runs with an empty `--cursor`, so it adds no
    /// prefix of its own and an option starts exactly at this margin.
    fn padded(&self, options: &[String]) -> Vec<String> {
        let widest = options.iter().map(|option| display_width(option)).max();
        let pad = " ".repeat(usize::from(self.block_margin(widest.unwrap_or(0))));
        options
            .iter()
            .map(|option| format!("{pad}{option}"))
            .collect()
    }
}

impl Menu for GumMenu {
    fn choose(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        height: u8,
    ) -> Result<Option<String>> {
        // `gum choose` draws one row per option and never pads out to `--height`.
        let rows = u16::try_from(options.len()).unwrap_or(u16::MAX);
        self.render_banner(title, subtitle, rows.min(u16::from(height)))?;
        let mut args = vec![
            "choose".to_owned(),
            "--height".to_owned(),
            height.to_string(),
            "--no-show-help".to_owned(),
            "--cursor".to_owned(),
            String::new(),
            "--header".to_owned(),
            String::new(),
        ];
        args.extend(self.padded(options));
        gum_output(args)
    }

    fn filter(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        placeholder: &str,
    ) -> Result<Option<String>> {
        // Unlike `choose`, `gum filter`'s `--height` is the whole frame: the query line,
        // the list, and the help line. It always occupies that many rows.
        self.render_banner(title, subtitle, FILTER_HEIGHT)?;
        // --no-strict returns the typed text when it matches no branch, so the same field
        // both picks an existing branch and names a new one.
        gum_with_input(
            &[
                "filter",
                "--no-strict",
                "--height",
                FILTER_HEIGHT_ARG,
                "--placeholder",
                placeholder,
            ],
            &self.padded(options).join("\n"),
        )
    }

    fn input(
        &mut self,
        title: &str,
        subtitle: &str,
        placeholder: &str,
        width: u16,
        indent: InputIndent,
    ) -> Result<Option<String>> {
        self.render_banner(title, subtitle, 1)?;
        if indent == InputIndent::Centered {
            print!("{}", " ".repeat(usize::from(self.block_margin(width))));
            io::stdout().flush().context("failed to indent gum input")?;
        }
        gum_output([
            "input",
            "--placeholder",
            placeholder,
            "--width",
            &width.to_string(),
        ])
    }
}

/// Run `gum` and capture its selection.
///
/// `gum` writes the selection to stdout but draws its UI on *stderr* whenever stdout
/// is not a terminal — which is exactly our case, since we capture the selection. So
/// stderr must be inherited or the menu renders nowhere and the user chooses blind.
/// A non-zero exit means the user cancelled: `Ok(None)`, never an error.
fn gum_output<I, S>(args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("gum")
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run gum")?;
    Ok(output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }))
}

/// Same contract as [`gum_output`], for the filter menu whose options arrive on stdin.
fn gum_with_input(args: &[&str], input: &str) -> Result<Option<String>> {
    let mut child = Command::new("gum")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to run gum")?;
    child
        .stdin
        .take()
        .context("failed to open gum input")?
        .write_all(input.as_bytes())
        .context("failed to write gum options")?;
    let output = child
        .wait_with_output()
        .context("failed to read gum selection")?;
    Ok(output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }))
}

#[cfg(test)]
mod popup {
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::herdr::FakeClient;

    fn popup_choice() -> AgentChoice {
        AgentChoice {
            label: "\u{f4af}  review".to_owned(),
            project_dir: PathBuf::from("/projects/example"),
            branch: None,
            launch: vec!["codex".to_owned(), "--profile work".to_owned()],
            harness: HARNESS_CODEX.to_owned(),
            model_label: None,
        }
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

    fn launch_options(tab_id: Option<&str>, no_layout: bool) -> LaunchOptions {
        LaunchOptions {
            pane_id: "p1".to_owned(),
            tab_id: tab_id.map(str::to_owned),
            usage: None,
            worktree: false,
            no_layout,
            restart: false,
        }
    }

    #[test]
    fn launcher_builds_the_required_layout_sequence() {
        let client = FakeClient::default();
        queue_popup_splits(&client);

        apply_launch_layout(&client, &launch_options(Some("t1"), false), &popup_choice()).unwrap();

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
                    json!({ "pane_id": "p2", "label": FILES_LABEL })
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
                    json!({ "pane_id": "p3", "label": TERM_LABEL })
                ),
            ]
        );
    }

    #[test]
    fn no_layout_skips_splits_and_tab_rename_is_optional() {
        let client = FakeClient::default();

        apply_launch_layout(&client, &launch_options(None, true), &popup_choice()).unwrap();

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
        };

        inject(&client, &options).unwrap();

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
        ];
        assert_eq!(argv.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn popup_reproduces_the_exact_ten_call_sequence() {
        let client = FakeClient::default();
        queue_popup_create(&client);
        queue_popup_splits(&client);

        create_popup_tab(&client, &popup_choice(), None).unwrap();

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
                    json!({ "pane_id": "p2", "label": FILES_LABEL })
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
                    json!({ "pane_id": "p3", "label": TERM_LABEL })
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
            create_popup_tab(&client, &popup_choice(), workspace).unwrap();
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

            let error = create_popup_tab(&client, &popup_choice(), None).unwrap_err();
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
            create_popup_tab(&client, &choice, None).unwrap();
        }
        assert!(client.calls.into_inner().is_empty());
    }

    #[test]
    fn popup_extra_args_preserve_toml_array_boundaries_and_bypass_is_opt_in() {
        let mut config = config();
        config.agents[1].extra_args = Vec::new();
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
            ["codex"]
        );
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(build_launch(&config, HARNESS_CODEX, None).unwrap().len(), 2);
        config.agents[1].extra_args = ["--search", "--profile", "work"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(build_launch(&config, HARNESS_CODEX, None).unwrap().len(), 4);
        config.agents[1].extra_args = vec!["--profile work".to_owned()];
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
            ["codex", "--profile work"]
        );
    }

    fn config() -> Config {
        Config {
            dashboard_workspace: String::new(),
            default_tab_layout: String::new(),
            project_registry_file: String::new(),
            projects_root: String::new(),
            ssh_registry_file: String::new(),
            ssh_config_file: String::new(),
            ssh_history_file: String::new(),
            tab_layouts: Vec::new(),
            agents: vec![
                Agent {
                    name: "claude code".to_owned(),
                    label: None,
                    icon: None,
                    command: vec!["claude".to_owned()],
                    extra_args: vec!["argument with space".to_owned()],
                    options: vec![
                        AgentOption {
                            name: "Opus".to_owned(),
                            args: ["--model", "claude-opus-4-8"].map(str::to_owned).to_vec(),
                            command: None,
                        },
                        AgentOption {
                            name: "OpusPlan (Sonnet)".to_owned(),
                            args: ["--model", "opusplan", "--effort", "medium"]
                                .map(str::to_owned)
                                .to_vec(),
                            command: None,
                        },
                        AgentOption {
                            name: "CCR".to_owned(),
                            args: Vec::new(),
                            command: Some(["ccr", "code"].map(str::to_owned).to_vec()),
                        },
                        AgentOption {
                            name: "Fable 5".to_owned(),
                            args: ["--model", "claude-fable-5"].map(str::to_owned).to_vec(),
                            command: None,
                        },
                    ],
                },
                Agent {
                    name: "codex".to_owned(),
                    label: None,
                    icon: None,
                    command: vec!["codex".to_owned()],
                    extra_args: vec!["--search".to_owned()],
                    options: Vec::new(),
                },
                Agent {
                    name: "opencode".to_owned(),
                    label: None,
                    icon: None,
                    command: vec!["opencode".to_owned()],
                    extra_args: Vec::new(),
                    options: Vec::new(),
                },
            ],
        }
    }

    /// Replays scripted answers in menu order, so a test can cancel at an exact step.
    struct FakeMenu {
        answers: VecDeque<Option<String>>,
        options: Vec<Vec<String>>,
    }

    impl FakeMenu {
        fn new<const N: usize>(answers: [Option<&str>; N]) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
                options: Vec::new(),
            }
        }

        fn answered_everything(&self) -> bool {
            self.answers.is_empty()
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
    fn use_last_is_first_and_skips_model_and_usage_menus() {
        let config = config();
        let entry = format!("{USE_LAST_PREFIX}claude code · Opus");
        let mut menu = FakeMenu::new([Some(entry.as_str())]);

        let choice = choose_agent_with_last(
            &config,
            Path::new("/project"),
            false,
            Some("review"),
            Some((HARNESS_CLAUDE.to_owned(), Some("Opus".to_owned()))),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options.len(), 1);
        assert_eq!(menu.options[0][0], entry);
        assert_eq!(choice.harness, HARNESS_CLAUDE);
        assert_eq!(choice.model_label.as_deref(), Some("Opus"));
    }

    #[test]
    fn stale_last_choice_does_not_add_a_menu_entry() {
        let config = config();
        let mut menu = FakeMenu::new([Some(HARNESS_CODEX)]);

        let choice = choose_agent_with_last(
            &config,
            Path::new("/project"),
            false,
            Some("review"),
            Some((HARNESS_CLAUDE.to_owned(), Some("Removed".to_owned()))),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options[0][0], HARNESS_CLAUDE);
        assert_eq!(choice.harness, HARNESS_CODEX);
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
        let config = config();
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
            ["codex", "--search"]
        );
        assert_eq!(
            build_launch(&config, HARNESS_OPENCODE, None).unwrap(),
            ["opencode"]
        );
        // A command override replaces the agent command, option args, and extra args.
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("CCR")).unwrap(),
            ["ccr", "code"]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("Opus")).unwrap(),
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "argument with space"
            ]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("OpusPlan (Sonnet)")).unwrap(),
            [
                "claude",
                "--model",
                "opusplan",
                "--effort",
                "medium",
                "argument with space"
            ]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("Fable 5")).unwrap(),
            ["claude", "--model", "claude-fable-5", "argument with space"]
        );
    }

    #[test]
    fn an_extra_argument_containing_a_space_stays_one_entry() {
        let mut config = config();
        config.agents[0].extra_args = vec![
            "--add-dir".to_owned(),
            "/Users/q/My Projects".to_owned(),
            "--dangerously-skip-permissions".to_owned(),
        ];
        config.agents[1].extra_args = vec!["--cd".to_owned(), "/Users/q/My Projects".to_owned()];

        let claude = build_launch(&config, HARNESS_CLAUDE, Some("Opus")).unwrap();
        assert_eq!(
            claude,
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "--add-dir",
                "/Users/q/My Projects",
                "--dangerously-skip-permissions"
            ]
        );
        let codex = build_launch(&config, HARNESS_CODEX, None).unwrap();
        assert_eq!(codex, ["codex", "--cd", "/Users/q/My Projects"]);
    }

    #[test]
    fn bypass_flags_are_absent_unless_configured() {
        let mut config = config();
        config.agents[0].extra_args = Vec::new();
        config.agents[1].extra_args = Vec::new();
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("Opus")).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
            ["codex"]
        );

        config.agents[0].extra_args = vec!["--dangerously-skip-permissions".to_owned()];
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("Opus")).unwrap(),
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "--dangerously-skip-permissions"
            ]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
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
            Some(HARNESS_CODEX),
            Some(USAGE_DISCUSS),
        ]);
        let with_worktree = choose_agent_with(
            &config,
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

        let mut menu = FakeMenu::new([Some(HARNESS_CODEX), Some(USAGE_DISCUSS)]);
        let never_a_worktree = choose_agent_with(
            &config,
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
            harness: HARNESS_CODEX.to_owned(),
            model_label: None,
        };
        let normalised = without_worktree(choice, Path::new("/projects/example"));
        assert_eq!(normalised.label, "review menu");
    }

    #[test]
    fn the_free_text_usage_path_names_the_tab() {
        let config = config();
        let mut menu = FakeMenu::new([
            Some(HARNESS_OPENCODE),
            Some(USAGE_WRITE),
            Some("ship the rewrite"),
        ]);
        let choice = choose_agent_with(
            &config,
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
        assert_eq!(choice.harness, HARNESS_OPENCODE);
        assert_eq!(choice.model_label, None);
    }

    #[test]
    fn a_fixed_usage_skips_the_usage_menu_and_is_used_verbatim() {
        let config = config();

        // Only the harness answer is scripted: a usage menu would read past the end and
        // cancel the flow.
        let mut menu = FakeMenu::new([Some(HARNESS_CODEX)]);
        let choice = choose_agent_with(
            &config,
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

        let mut menu = FakeMenu::new([Some(HARNESS_CLAUDE), Some("Opus")]);
        let choice = choose_agent_with(
            &config,
            Path::new("/projects/example"),
            false,
            Some("\u{f442}  discuss"),
            &mut menu,
            &FakeGit::repository("/projects/example"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(choice.label, "\u{f442}  discuss");
        assert_eq!(choice.model_label.as_deref(), Some("Opus"));
        assert!(menu.answered_everything());
    }

    #[test]
    fn an_empty_branch_name_becomes_a_timestamped_one() {
        let config = config();
        let mut menu = FakeMenu::new([Some("   "), Some(HARNESS_CODEX), Some(USAGE_DEBUG)]);
        let choice = choose_agent_with(
            &config,
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
                vec![Some("feature/menu"), Some(HARNESS_CLAUDE), None],
            ),
            (
                "usage",
                vec![
                    Some("feature/menu"),
                    Some(HARNESS_CLAUDE),
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
            };
            let choice = choose_agent_with(
                &config,
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
                vec![Some("feature/menu"), Some(HARNESS_CLAUDE), None],
            ),
            (
                "usage",
                vec![
                    Some("feature/menu"),
                    Some(HARNESS_CLAUDE),
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
            };
            let choice =
                choose_agent_with(&config, &fixture.repo(), true, None, &mut menu, &RealGit)
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
            Some(HARNESS_CODEX),
            Some(USAGE_REVIEW),
        ]);
        let choice = choose_agent_with(&config, &fixture.repo(), true, None, &mut menu, &RealGit)
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
    fn the_centering_geometry_matches_the_popup() {
        let menu = GumMenu::new(80, 40);
        assert_eq!(menu.content_width(), 44);
        // `--width 44` draws 46 columns, so 17 either side of an 80-column viewport.
        assert_eq!(menu.content_margin(), 17);
        assert_eq!(80 - menu.content_margin() - (menu.content_width() + 2), 17);
        // A 24-column block leaves 28 either side; the old fixed choice margin.
        assert_eq!(menu.block_margin(24), 28);
        assert_eq!(
            menu.padded(&[USAGE_DEBUG.to_owned()]),
            [format!("{}{USAGE_DEBUG}", " ".repeat(36))]
        );

        // A viewport too narrow for the banner narrows the box and floors both margins.
        let menu = GumMenu::new(20, 8);
        assert_eq!(menu.content_width(), 16);
        assert_eq!(menu.content_margin(), 1);
        assert_eq!(menu.block_margin(24), 0);
    }

    #[test]
    fn the_block_is_centered_on_the_rows_it_occupies() {
        // The banner is 7 rows, plus the blank line, plus the menu body.
        let menu = GumMenu::new(80, 40);
        assert_eq!(menu.vertical_padding(7 + 1 + 3), 14);
        assert_eq!(40 - menu.vertical_padding(11) - 11, 15);
        // The worktree filter reserves its whole frame, so it sits higher.
        assert_eq!(menu.vertical_padding(7 + 1 + FILTER_HEIGHT), 10);

        // A viewport shorter than the block floors the padding instead of scrolling.
        assert_eq!(GumMenu::new(80, 10).vertical_padding(11), 0);
    }

    #[test]
    fn the_option_block_is_centered_on_its_widest_option() {
        let menu = GumMenu::new(80, 40);
        let options = [
            HARNESS_CLAUDE.to_owned(),
            HARNESS_CODEX.to_owned(),
            HARNESS_OPENCODE.to_owned(),
        ];
        // The widest option is 14 columns, so the block starts at 33 and ends at 47.
        let padded = menu.padded(&options);
        let pad = " ".repeat(33);
        assert!(
            padded.iter().all(|option| option.starts_with(&pad)),
            "every option shares one left edge so the glyphs line up: {padded:?}"
        );
        assert_eq!(display_width(&padded[0]), 47);
        assert_eq!(80 - display_width(&padded[0]), 33);

        // A narrower option set moves further right; the old fixed 24 could not.
        let model = menu.padded(&["Fable 5".to_owned()]);
        assert_eq!(display_width(&model[0]) - 7, 36);
    }

    #[test]
    fn a_wide_character_counts_as_two_columns() {
        // Measured against `gum style --border rounded`, which draws the same widths.
        assert_eq!(display_width("中文分支"), 8);
        assert_eq!(display_width("こんにちは"), 10);
        assert_eq!(display_width("feat/中文"), 9);
        // A Nerd Font glyph is one column, so these labels measure as plain text.
        assert_eq!(display_width(HARNESS_CLAUDE), 14);
        assert_eq!(display_width(USAGE_WRITE), 16);
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
