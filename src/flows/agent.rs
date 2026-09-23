use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::config::{render_label, Agent, AgentOption, Config, LayoutPane, PaneType, TabLayout};
use crate::flows::menu::{popup_viewport, strip_pad, GumMenu, InputIndent, Menu, ModelRow};
use crate::flows::{
    invoking_pane_cwd, nonempty_env, session, FlowError, FlowResult, Outcome, PaneCwd,
};
use crate::herdr::types::{ErrorResponse, LayoutNode};
use crate::herdr::HerdrClient;
use crate::shell::build_command;
use crate::state;

/// Herdr's refusal when a pane is not yet sitting at an interactive shell prompt.
const PANE_BUSY: &str = "agent_pane_busy";
const NAME_TAKEN: &str = "agent_name_taken";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const START_RETRY: Duration = Duration::from_millis(250);

const HARNESS_TITLE: &str = "\u{f169f}  Launch Agent";
const USE_LAST_PREFIX: &str = "\u{f0709}  use last: ";
// Two spaces after the glyph. `scripts/agent-launcher.zsh:183` used one; the unified
// flow follows the popup and the parity contract (GLY-2).
const USAGE_TITLE: &str = "\u{f27b}  Usage";
// A tab that runs no harness is named, not classified by usage.
const TAB_NAME_TITLE: &str = "\u{eb03}  Tab Name";
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
    /// The session the restarted harness should pick up. Only the pane being restarted
    /// resumes; any other agent pane the layout builds still starts fresh.
    pub resume: Option<String>,
    pub layout: Option<String>,
    /// Which agent pane of the layout this launch is. Restart names the pane it is
    /// replacing; without it the layout's first agent pane drives the launch.
    pub pane: Option<String>,
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
    let target = launch_target(layout, options)?;
    // Building the layout means creating its other agent panes too, so every one of them
    // has to be decided here. A launch that builds nothing asks about the target alone.
    let panes = if options.no_layout {
        vec![target]
    } else {
        layout.agent_panes().map(|(_, pane)| pane).collect()
    };
    let Some(mut choice) = choose_agent(
        config,
        layout,
        &panes,
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
    let mut launch = choice
        .agent(Some(&target.name))
        .expect("the target pane was asked about")
        .clone();
    // Rebuilt rather than threaded through the menus: resume belongs to this one pane,
    // and the menus decide for every agent pane the layout builds.
    if let Some(session) = &options.resume {
        launch.launch = build_launch(
            config,
            &launch.agent_name,
            launch.option_name.as_deref(),
            launch.effort.as_deref(),
            Some(session),
        )?;
    }
    apply_launch_layout(client, layout, options, &choice)?;
    std::env::set_current_dir(&choice.project_dir)
        .with_context(|| format!("failed to enter {}", choice.project_dir.display()))?;
    Command::new("clear")
        .status()
        .context("failed to clear the terminal before launching the agent")?;

    // Narrowest possible window: the harness this pane is about to become writes its
    // session file after the exec below.
    let since_ms = session::now_ms();
    let mut record = last_agent_record(&launch, layout)?;
    // A resume already knows its session, and losing it here would leave the next resume
    // with nothing to fall back on when the reporter finds no file.
    record.session = options.resume.clone();
    let _ = state::write_state(client, &options.pane_id, &record);
    // exec destroys this process, so the only moment a reporter can be started is here.
    if let Some(report) = reporter_options(&launch, &choice, &options.pane_id, since_ms) {
        let _ = session::spawn_reporter(&report);
    }

    // A child wrapper breaks restart-in-place. exec only returns when execvp fails.
    let error = Command::new(&launch.launch[0])
        .args(&launch.launch[1..])
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

/// The layout pane this launch replaces its own process with.
///
/// A launch that also builds the layout treats the invoking pane as the tab root, so the
/// root has to be the agent pane; the popup path is the one that can put a harness in a
/// split. A launch that builds nothing — restart — runs in whichever pane it names, so it
/// carries no root requirement.
fn launch_target<'a>(layout: &'a TabLayout, options: &LaunchOptions) -> Result<&'a LayoutPane> {
    let Some((index, pane)) = layout.agent_pane(options.pane.as_deref()) else {
        match &options.pane {
            Some(name) => bail!(
                "layout '{}' has no agent pane named '{}'",
                layout.name,
                name
            ),
            None => bail!(
                "layout '{}' declares no agent pane, so there is no harness to launch",
                layout.name
            ),
        }
    };
    if !options.no_layout && index != 0 {
        bail!(
            "layout '{}': agent pane '{}' is not the tab root, so it can only be opened from the popup",
            layout.name,
            pane.name
        );
    }
    Ok(pane)
}

fn apply_launch_layout(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    options: &LaunchOptions,
    choice: &AgentChoice,
) -> Result<()> {
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
    build_side_panes(client, layout, choice, &options.pane_id)?;
    Ok(())
}

fn build_side_panes(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    root_pane: &str,
) -> Result<()> {
    let cwd = choice.project_dir.to_string_lossy();
    let cwd = cwd.as_ref();
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
        let label = pane_label(pane, choice);
        if let Some(label) = &label {
            client
                .pane_rename(json!({ "pane_id": pane_id, "label": label }))
                .with_context(|| format!("failed to rename pane {}", pane.name))?;
        }
        start_pane(
            client,
            layout,
            pane,
            &pane_id,
            choice,
            label.as_deref().unwrap_or(&pane.name),
        )?;
        pane_ids.insert(pane.name.as_str(), pane_id.clone());
        previous_pane_id = pane_id;
    }
    Ok(())
}

/// The label a pane of a whole tab wears: the tab root wears the usage label the tab
/// itself is named after, every other pane its own.
fn tab_pane_label(index: usize, pane: &LayoutPane, choice: &AgentChoice) -> Option<String> {
    match index {
        0 => Some(choice.label.clone()),
        _ => pane_label(pane, choice),
    }
}

/// Herdr names an agent `[a-z][a-z0-9_-]{0,31}`, which a pane label is not: labels carry
/// capitals, spaces and Nerd Font glyphs, and handing one over is refused outright.
fn agent_name(label: &str, kind: &str) -> String {
    let mut name = String::new();
    for character in label.chars().flat_map(char::to_lowercase) {
        let candidate = match character {
            'a'..='z' | '0'..='9' | '-' | '_' => character,
            _ => '-',
        };
        // Nothing before the first letter can start a name, and one separator says as much
        // as a run of them.
        if (name.is_empty() && !candidate.is_ascii_lowercase())
            || (candidate == '-' && name.ends_with('-'))
        {
            continue;
        }
        name.push(candidate);
        if name.len() == 32 {
            break;
        }
    }
    let name = name.trim_end_matches(['-', '_']);
    match name.is_empty() {
        // A kind is one of Herdr's own identifiers, so it always satisfies the rule.
        true => kind.to_owned(),
        false => name.to_owned(),
    }
}

/// `base-number`, truncating the base so the whole still fits Herdr's 32 characters.
fn numbered_agent_name(base: &str, number: usize) -> String {
    let suffix = format!("-{number}");
    let base = &base[..base.len().min(32 - suffix.len())];
    format!("{}{suffix}", base.trim_end_matches(['-', '_']))
}

/// A pane whose shell has not reached its prompt is refused, and a real profile takes
/// seconds to get there, so that refusal is retried as is. A name another agent already
/// holds — two tabs opened under one usage label — is retried under a numbered name.
fn start_agent(client: &dyn HerdrClient, mut params: Value) -> Result<()> {
    let deadline = Instant::now() + START_TIMEOUT;
    let base = params["name"].as_str().unwrap_or_default().to_owned();
    let mut number = 1;
    loop {
        let error = match client.agent_start(params.clone()) {
            Ok(_) => return Ok(()),
            Err(error) => error,
        };
        let code = error
            .downcast_ref::<ErrorResponse>()
            .map(|response| response.code.as_str());
        if Instant::now() >= deadline {
            return Err(error);
        }
        match code {
            Some(PANE_BUSY) => thread::sleep(START_RETRY),
            Some(NAME_TAKEN) => {
                number += 1;
                params["name"] = json!(numbered_agent_name(&base, number));
            }
            _ => return Err(error),
        }
    }
}

/// Start what one pane runs, and record an agent pane's choice for restart.
///
/// Herdr starts a harness it knows the kind of, which is what makes the pane an agent it
/// can see; everything else — a command pane's shell line, or an option that overrides the
/// executable — is typed into the pane's interactive shell instead. `name` is the label the
/// pane already wears, so an agent Herdr starts is listed under the same name.
fn start_pane(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    pane: &LayoutPane,
    pane_id: &str,
    choice: &AgentChoice,
    name: &str,
) -> Result<()> {
    if pane.pane_type == PaneType::Agent {
        let Some(agent) = choice.agent(Some(&pane.name)) else {
            return Ok(());
        };
        // Read before the harness starts: a transcript written during startup would
        // otherwise be older than the window the reporter searches.
        let since_ms = session::now_ms();
        match &agent.start {
            Some(start) => {
                start_agent(
                    client,
                    json!({
                        "pane_id": pane_id,
                        "name": agent_name(name, &start.kind),
                        "kind": start.kind,
                        "args": start.args,
                    }),
                )
                .with_context(|| format!("failed to start the agent in pane {}", pane.name))?;
            }
            None => send_pane_input(client, pane_id, &build_command(&agent.launch), &pane.name)?,
        }
        let record = last_agent_record(agent, layout)?;
        let _ = state::write_state(client, pane_id, &record);
        // Every agent pane the plugin does not `exec` into reports from here; the one it
        // does reports from `launch`, which has no process left after the exec.
        if let Some(options) = reporter_options(agent, choice, pane_id, since_ms) {
            let _ = session::spawn_reporter(&options);
        }
        return Ok(());
    }
    if let (PaneType::Command, Some(command)) = (pane.pane_type, &pane.command) {
        send_pane_input(client, pane_id, command, &pane.name)?;
    }
    Ok(())
}

/// What the session reporter needs for one agent pane, or None for an agent whose kind
/// names no session files to poll.
fn reporter_options(
    agent: &PaneAgent,
    choice: &AgentChoice,
    pane_id: &str,
    since_ms: u64,
) -> Option<session::ReportOptions> {
    Some(session::ReportOptions {
        pane_id: pane_id.to_owned(),
        agent: agent.agent_name.clone(),
        kind: agent.kind.clone()?,
        cwd: choice.project_dir.clone(),
        since_ms,
    })
}

fn send_pane_input(
    client: &dyn HerdrClient,
    pane_id: &str,
    text: &str,
    pane_name: &str,
) -> Result<()> {
    client
        .pane_send_input(json!({
            "pane_id": pane_id,
            "text": text,
            "keys": ["enter"],
        }))
        .with_context(|| format!("failed to start command in pane {pane_name}"))?;
    Ok(())
}

fn pane_label(pane: &LayoutPane, choice: &AgentChoice) -> Option<String> {
    // An agent pane that names no label says which harness it is running instead;
    // "codex" beside "claude code" is what tells two agent panes apart.
    let fallback = (pane.pane_type == PaneType::Agent)
        .then(|| choice.agent(Some(&pane.name)))
        .flatten()
        .map(|agent| render_label(pane.icon.as_deref(), &agent.agent_name));
    pane.label
        .as_deref()
        .map(|label| render_label(pane.icon.as_deref(), label))
        .or(fallback)
}

/// One node of the fold before it becomes a `LayoutNode`; a leaf names the layout pane it
/// stands for, and a split refers back into the arena that holds it.
enum FoldNode {
    Leaf(usize),
    Split {
        direction: &'static str,
        ratio: f64,
        first: usize,
        second: usize,
    },
}

/// Fold a layout's flat pane list into the tree `layout.apply` takes, alongside its leaves'
/// layout pane names in traversal order.
///
/// Each pane takes the place of the leaf it splits from, pushing that leaf down as the
/// split's first child. `split_from` is validated at load to name an earlier pane, so no
/// step can miss its slot or close a cycle.
fn build_layout_tree<'a>(
    layout: &'a TabLayout,
    choice: &AgentChoice,
    cwd: &str,
) -> (LayoutNode, Vec<&'a str>) {
    let mut arena = vec![FoldNode::Leaf(0)];
    let mut slots = BTreeMap::from([(layout.panes[0].name.as_str(), 0usize)]);
    let mut previous = layout.panes[0].name.as_str();
    for (index, pane) in layout.panes.iter().enumerate().skip(1) {
        let target = pane.split_from.as_deref().unwrap_or(previous);
        let slot = *slots.get(target).expect("validated at load");
        let FoldNode::Leaf(target_index) = arena[slot] else {
            unreachable!("a slot only ever holds a leaf");
        };
        let moved = arena.len();
        arena.push(FoldNode::Leaf(target_index));
        let added = arena.len();
        arena.push(FoldNode::Leaf(index));
        arena[slot] = FoldNode::Split {
            direction: match pane.direction.expect("validated at load") {
                crate::config::Direction::Right => "right",
                crate::config::Direction::Down => "down",
            },
            // Config ratios are self-describing as each new pane's share, while Herdr's
            // ratio is the original pane's share after the split. For example, `files`
            // uses 0.62 so Files takes 62%, the agent keeps 38%, and Herdr receives 38%.
            ratio: 1.0 - pane.ratio.expect("validated at load"),
            first: moved,
            second: added,
        };
        slots.insert(target, moved);
        slots.insert(pane.name.as_str(), added);
        previous = pane.name.as_str();
    }
    let mut names = Vec::with_capacity(layout.panes.len());
    let root = materialise_layout(&arena, 0, layout, choice, cwd, &mut names);
    (root, names)
}

fn materialise_layout<'a>(
    arena: &[FoldNode],
    index: usize,
    layout: &'a TabLayout,
    choice: &AgentChoice,
    cwd: &str,
    names: &mut Vec<&'a str>,
) -> LayoutNode {
    match &arena[index] {
        FoldNode::Leaf(pane_index) => {
            let pane = &layout.panes[*pane_index];
            names.push(pane.name.as_str());
            LayoutNode::Pane {
                pane_id: None,
                cwd: Some(cwd.to_owned()),
                env: pane.env.clone(),
                label: tab_pane_label(*pane_index, pane, choice),
                command: None,
            }
        }
        FoldNode::Split {
            direction,
            ratio,
            first,
            second,
        } => LayoutNode::Split {
            direction: (*direction).to_owned(),
            ratio: *ratio,
            first: Box::new(materialise_layout(
                arena, *first, layout, choice, cwd, names,
            )),
            second: Box::new(materialise_layout(
                arena, *second, layout, choice, cwd, names,
            )),
        },
    }
}

/// Every leaf of a Herdr layout tree, left to right.
fn leaf_pane_ids(node: &LayoutNode) -> Vec<String> {
    match node {
        LayoutNode::Pane { pane_id, .. } => vec![pane_id.clone().unwrap_or_default()],
        LayoutNode::Split { first, second, .. } => {
            let mut ids = leaf_pane_ids(first);
            ids.extend(leaf_pane_ids(second));
            ids
        }
    }
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
    let panes = layout
        .agent_panes()
        .map(|(_, pane)| pane)
        .collect::<Vec<_>>();
    let Some(mut choice) = choose_agent(
        config, layout, &panes, &cwd, worktree, None, cols, lines, None,
    )?
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

/// Build the tab's whole structure in one `layout.apply`, then start what each pane runs.
///
/// Only apply is atomic, so only its failure leaves nothing to close; a harness that never
/// becomes ready would otherwise strand a built tab with no agent in it.
fn create_popup_tab(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    workspace_id: Option<String>,
) -> Result<()> {
    let applied = apply_popup_layout(client, layout, choice, workspace_id)
        .map_err(|error| FlowError::titled("Agent tab failed", error))?;
    if let Err(error) = fill_popup_tab(client, layout, choice, &applied) {
        let _ = client.tab_close(json!({ "tab_id": applied.tab_id }));
        return Err(FlowError::prefixed(
            "Agent tab failed",
            "The incomplete tab was closed.",
            error,
        )
        .into());
    }
    Ok(())
}

/// The tab `layout.apply` built, with Herdr's pane ids under our own layout pane names.
struct AppliedTab {
    tab_id: String,
    pane_ids: BTreeMap<String, String>,
}

fn apply_popup_layout(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    workspace_id: Option<String>,
) -> Result<AppliedTab> {
    let cwd = choice.project_dir.to_string_lossy();
    let (root, names) = build_layout_tree(layout, choice, cwd.as_ref());
    let mut params = Map::from_iter([
        (
            "root".to_owned(),
            serde_json::to_value(&root).context("failed to encode the tab layout")?,
        ),
        ("tab_label".to_owned(), json!(choice.label)),
        ("focus".to_owned(), json!(false)),
    ]);
    if let Some(workspace_id) = workspace_id {
        params.insert("workspace_id".to_owned(), json!(workspace_id));
    }
    let applied = client
        .layout_apply(Value::Object(params))
        .context("failed to create agent tab")?
        .layout;

    // Herdr's leaves carry none of our names, so traversal position is the only link back.
    let pane_ids = leaf_pane_ids(&applied.root);
    if pane_ids.len() != names.len() {
        return Err(anyhow!(
            "layout.apply returned {} panes for a layout of {} panes",
            pane_ids.len(),
            names.len()
        ));
    }
    if pane_ids.iter().any(String::is_empty) {
        return Err(anyhow!("layout.apply returned an empty pane id"));
    }
    Ok(AppliedTab {
        tab_id: applied.tab_id,
        pane_ids: names
            .into_iter()
            .map(str::to_owned)
            .zip(pane_ids)
            .collect::<BTreeMap<_, _>>(),
    })
}

fn fill_popup_tab(
    client: &dyn HerdrClient,
    layout: &TabLayout,
    choice: &AgentChoice,
    applied: &AppliedTab,
) -> Result<()> {
    // Before the panes start: `agent.start` waits for its harness to be ready, so focusing
    // afterwards would leave the caller on the old tab for as long as that takes.
    client
        .tab_focus(json!({ "tab_id": applied.tab_id }))
        .context("failed to focus agent tab")?;
    // The layout's own order, so two agent panes start in the order they are written.
    for (index, pane) in layout.panes.iter().enumerate() {
        let pane_id = &applied.pane_ids[pane.name.as_str()];
        let name = tab_pane_label(index, pane, choice).unwrap_or_else(|| pane.name.clone());
        start_pane(client, layout, pane, pane_id, choice, &name)?;
    }
    Ok(())
}

fn last_agent_record(agent: &PaneAgent, layout: &TabLayout) -> Result<state::LastAgentRecord> {
    Ok(state::LastAgentRecord {
        agent: agent.agent_name.clone(),
        option: agent.option_name.clone(),
        effort: agent.effort.clone(),
        layout: layout.name.clone(),
        pane: agent.pane.clone(),
        session: None,
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
    /// The tab label, and the label of an agent pane that declares none: the usage label,
    /// plus two spaces and the branch when a worktree was chosen.
    pub label: String,
    /// The worktree when one was chosen, else the repository toplevel or the cwd.
    pub project_dir: PathBuf,
    pub branch: Option<String>,
    /// One entry per agent pane the caller asked about, in layout order. Empty for a
    /// layout that declares no agent pane.
    pub agents: Vec<PaneAgent>,
}

/// The harness resolved for one agent pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAgent {
    /// The layout pane's `name`. Restart stores it so a side agent pane replays its own
    /// pin rather than the first agent pane's.
    pub pane: String,
    /// argv, ready for `exec` or for a pane command.
    pub launch: Vec<String>,
    /// Set when Herdr can start this harness itself; None keeps the typed-argv path.
    pub start: Option<AgentStart>,
    /// Set even when `start` is None, because an overridden command still writes the
    /// session files of its kind.
    pub kind: Option<String>,
    /// The [[agents]] entry's `name`, not its rendered label.
    pub agent_name: String,
    /// The chosen [[agents.options]] entry's `name`; None for an agent with no options.
    pub option_name: Option<String>,
    /// The effort that option launches at, already resolved; None when it takes none.
    pub effort: Option<String>,
}

/// What `agent.start` needs to run a harness in a pane sitting at its shell prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStart {
    /// Herdr's agent kind, which is also where it derives the executable from.
    pub kind: String,
    /// Everything after that executable: the option's args, then the agent's extra args.
    pub args: Vec<String>,
}

impl AgentChoice {
    /// The decision for one layout pane, or the first when `pane` is None.
    pub fn agent(&self, pane: Option<&str>) -> Option<&PaneAgent> {
        match pane {
            Some(pane) => self.agents.iter().find(|entry| entry.pane == pane),
            None => self.agents.first(),
        }
    }
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
    panes: &[&LayoutPane],
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
        panes,
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
    let panes = layout
        .agent_panes()
        .map(|(_, pane)| pane)
        .collect::<Vec<_>>();
    choose_agent_with_last(
        config,
        layout,
        &panes,
        cwd,
        worktree,
        fixed_usage,
        None,
        menu,
        git,
    )
}

#[allow(clippy::too_many_arguments)]
fn choose_agent_with_last(
    config: &Config,
    layout: &TabLayout,
    panes: &[&LayoutPane],
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

    // Every agent pane is decided before anything is created, so cancelling at the last
    // pane's model menu still costs nothing. A stored choice replays only when one pane
    // is being asked about: with several it names no particular one.
    let last = last
        .filter(|_| panes.len() == 1)
        .filter(|record| state::last_choice_is_valid(record, config));
    let qualify = panes.len() > 1;
    let mut agents = Vec::with_capacity(panes.len());
    for pane in panes {
        let Some(agent) = choose_pane_agent(config, pane, qualify, last.clone(), menu)? else {
            return Ok(None);
        };
        agents.push(agent);
    }

    // A fixed usage skips the menu and is used verbatim: the restart path passes the
    // pane's current label, and the project picker passes its pinned tab label. A layout
    // with no agent pane is not for a harness, so the usage question does not apply; it
    // asks for a plain tab name instead.
    let usage = match fixed_usage.or(layout.tab_label.as_deref()) {
        Some(usage) => usage.to_owned(),
        None if agents.is_empty() => match select_tab_name(layout, menu)? {
            Some(name) => name,
            None => return Ok(None),
        },
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

    Ok(Some(AgentChoice {
        label: compose_label(&usage, branch.as_deref()),
        project_dir,
        branch,
        agents,
    }))
}

/// Run the harness and model menus for one agent pane.
///
/// `qualify` appends the pane to both menu titles. A single-agent layout leaves them
/// exactly as they were, so the common flow is unchanged.
fn choose_pane_agent(
    config: &Config,
    pane: &LayoutPane,
    qualify: bool,
    last: Option<state::LastAgentRecord>,
    menu: &mut impl Menu,
) -> Result<Option<PaneAgent>> {
    let title = |base: &str| match qualify {
        true => format!("{base} · {}", pane.menu_label()),
        false => base.to_owned(),
    };

    // A pane that pins its agent runs no harness menu, so nothing here is built for it.
    let (agent_name, stored_option, stored_effort) = if let Some(agent_name) = &pane.agent {
        (agent_name.clone(), None, None)
    } else {
        let use_last = last.as_ref().map(|record| {
            let label = config
                .agent(&record.agent)
                .expect("validated by last_choice_is_valid")
                .menu_label();
            [record.option.as_deref(), record.effort.as_deref()]
                .into_iter()
                .flatten()
                .fold(format!("{USE_LAST_PREFIX}{label}"), |row, part| {
                    format!("{row} · {part}")
                })
        });
        let mut harness_options = config
            .agents
            .iter()
            .map(Agent::menu_label)
            .collect::<Vec<_>>();
        if let Some(option) = &use_last {
            harness_options.insert(0, option.clone());
        }
        let Some(harness) = menu.choose(
            &title(HARNESS_TITLE),
            "Choose a harness.",
            &harness_options,
            8,
        )?
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
            (record.agent, record.option, record.effort)
        } else {
            // Rendered labels are unique across agents, enforced at config load.
            let agent_name = config
                .agents
                .iter()
                .find(|agent| agent.menu_label() == harness)
                .map(|agent| agent.name.clone())
                .expect("validated at load");
            (agent_name, None, None)
        }
    };

    let agent = config.agent(&agent_name).expect("validated at load");
    let (option_name, requested_effort) =
        if let Some(option_name) = pane.option_name.clone().or(stored_option) {
            (Some(option_name), stored_effort)
        } else if agent.options.is_empty() {
            (None, None)
        } else {
            let rows = agent
                .options
                .iter()
                .map(|option| {
                    let efforts = match &option.effort {
                        Some(_) => agent.efforts_for(option).to_vec(),
                        None => Vec::new(),
                    };
                    let effort = efforts
                        .iter()
                        .position(|level| Some(level) == option.effort.as_ref())
                        .unwrap_or(0);
                    ModelRow {
                        label: option.name.clone(),
                        efforts,
                        effort,
                    }
                })
                .collect::<Vec<_>>();
            let subtitle = match rows.iter().any(|row| !row.efforts.is_empty()) {
                true => "Choose a model. ←/→ sets effort.",
                false => "Choose a model.",
            };
            let Some((index, effort)) =
                menu.choose_model(&title(&agent.menu_label()), subtitle, &rows)?
            else {
                return Ok(None);
            };
            (Some(agent.options[index].name.clone()), effort)
        };

    let effort = option_name
        .as_deref()
        .and_then(|name| agent.option(name))
        .and_then(|option| agent.resolve_effort(option, requested_effort.as_deref()));
    Ok(Some(PaneAgent {
        pane: pane.name.clone(),
        launch: build_launch(
            config,
            &agent_name,
            option_name.as_deref(),
            effort.as_deref(),
            None,
        )?,
        start: build_start(
            config,
            &agent_name,
            option_name.as_deref(),
            effort.as_deref(),
        ),
        kind: agent.kind.clone(),
        agent_name,
        option_name,
        effort,
    }))
}

/// Ask what to call a tab that runs no harness.
///
/// Submitting nothing keeps the layout's own label: a blank tab always has a usable name,
/// so an empty answer is a shrug rather than a cancellation. Escape still cancels, which
/// is why the empty string and `None` are not folded together the way [`select_usage`]
/// folds them.
fn select_tab_name(layout: &TabLayout, menu: &mut impl Menu) -> Result<Option<String>> {
    // A trailing ellipsis marks a menu row as opening a prompt; it belongs to the row, not
    // to the tab. Without this the blank layout would name its tab "Blank Tab…".
    let fallback = layout
        .menu_label()
        .trim_end_matches('\u{2026}')
        .trim_end()
        .to_owned();
    let Some(name) = menu.input(
        TAB_NAME_TITLE,
        "Name this tab.",
        &fallback,
        40,
        InputIndent::None,
    )?
    else {
        return Ok(None);
    };
    let name = name.trim();
    Ok(Some(match name.is_empty() {
        true => fallback,
        false => name.to_owned(),
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

/// `effort` is a request, resolved by [`Agent::resolve_effort`]; None asks for the option's
/// own default.
fn build_launch(
    config: &Config,
    agent_name: &str,
    option_name: Option<&str>,
    effort: Option<&str>,
    resume: Option<&str>,
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
    // Before the option args because codex takes a subcommand, which has to sit directly
    // after the executable; claude's flag form is indifferent to the position.
    if let Some(resume) = resume {
        launch.extend(resume_args(agent.kind.as_deref(), resume));
    }
    launch.extend(option.into_iter().flat_map(|option| option.args.clone()));
    launch.extend(option_effort_args(agent, option, effort));
    // A command override changes only the executable; extra args apply to every
    // launch of the agent, including overridden commands.
    launch.extend(agent.extra_args.clone());
    Ok(launch)
}

fn option_effort_args(
    agent: &Agent,
    option: Option<&AgentOption>,
    effort: Option<&str>,
) -> Vec<String> {
    option
        .and_then(|option| agent.resolve_effort(option, effort))
        .map(|effort| agent.effort_args_for(&effort))
        .unwrap_or_default()
}

fn resume_args(kind: Option<&str>, session: &str) -> Vec<String> {
    match kind {
        Some("claude") => vec!["--resume".to_owned(), session.to_owned()],
        Some("codex") => vec!["resume".to_owned(), session.to_owned()],
        _ => Vec::new(),
    }
}

/// Herdr's own integration reports sessions for kinds this plugin cannot resolve, so the
/// restart worker asks here before promising a resume it would drop.
pub(crate) fn kind_can_resume(kind: Option<&str>) -> bool {
    !resume_args(kind, "session").is_empty()
}

/// How Herdr would start this harness, or None when only typed argv can: `agent.start`
/// appends its args to an executable derived from the kind, so an option that overrides
/// the command has nowhere to put that override. Unknown names are [`build_launch`]'s to
/// report, and every caller runs it first.
fn build_start(
    config: &Config,
    agent_name: &str,
    option_name: Option<&str>,
    effort: Option<&str>,
) -> Option<AgentStart> {
    let agent = config.agent(agent_name)?;
    let kind = agent.kind.as_deref()?;
    // The same reason covers the agent's own command: an executable the kind does not name,
    // or fixed arguments sitting before the option's, would be dropped on the way.
    if agent.command.len() != 1 || agent.command[0] != kind {
        return None;
    }
    let option = option_name.and_then(|name| agent.option(name));
    if option.is_some_and(|option| option.command.is_some()) {
        return None;
    }
    let mut args = option.map(|option| option.args.clone()).unwrap_or_default();
    args.extend(option_effort_args(agent, option, effort));
    args.extend(agent.extra_args.clone());
    Some(AgentStart {
        kind: kind.to_owned(),
        args,
    })
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
            agents: vec![pane_agent("agent", "codex")],
        }
    }

    fn pane_agent(pane: &str, agent: &str) -> PaneAgent {
        PaneAgent {
            pane: pane.to_owned(),
            launch: vec![agent.to_owned(), "--profile work".to_owned()],
            start: None,
            kind: None,
            agent_name: agent.to_owned(),
            option_name: None,
            effort: None,
        }
    }

    fn default_layout() -> TabLayout {
        let config = Config::test_default();
        config.layout(&config.default_tab_layout).unwrap().clone()
    }

    /// Herdr's reply for the shipped layout: the tree it built, carrying pane ids and none
    /// of our pane names.
    fn queue_popup_apply(client: &FakeClient) {
        client.queue_response(
            "layout.apply",
            json!({
                "type": "layout_apply",
                "layout": {
                    "workspace_id": "w1",
                    "tab_id": "t1",
                    "zoomed": false,
                    "focused_pane_id": "p1",
                    "root": {
                        "type": "split", "direction": "right", "ratio": 0.38,
                        "first": { "type": "pane", "pane_id": "p1" },
                        "second": {
                            "type": "split", "direction": "down", "ratio": 0.9,
                            "first": { "type": "pane", "pane_id": "p2" },
                            "second": { "type": "pane", "pane_id": "p3" },
                        },
                    },
                },
            }),
        );
    }

    fn queue_popup_splits(client: &FakeClient) {
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p2" } }));
        client.queue_response("pane.split", json!({ "pane": { "pane_id": "p3" } }));
    }

    #[test]
    fn the_shipped_layout_folds_into_a_right_split_over_a_down_split() {
        let layout = default_layout();

        let (root, leaves) = build_layout_tree(&layout, &popup_choice(), "/projects/example");

        assert_eq!(leaves, ["agent", "files", "term"]);
        let LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } = &root
        else {
            panic!("expected a split at the root: {root:?}");
        };
        assert_eq!(direction, "right");
        // Config's 0.62 describes `files`; Herdr's ratio is the agent's remaining share.
        assert_eq!(*ratio, 0.38);
        assert_eq!(
            **first,
            LayoutNode::Pane {
                pane_id: None,
                cwd: Some("/projects/example".to_owned()),
                env: BTreeMap::from([("Q_NO_BANNER".to_owned(), "1".to_owned())]),
                label: Some("\u{f4af}  review".to_owned()),
                command: None,
            }
        );
        let LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } = &**second
        else {
            panic!("expected `term` to split `files`: {second:?}");
        };
        assert_eq!(direction, "down");
        assert_eq!(*ratio, 0.9);
        assert_eq!(
            **first,
            LayoutNode::Pane {
                pane_id: None,
                cwd: Some("/projects/example".to_owned()),
                env: BTreeMap::from([("Q_NO_BANNER".to_owned(), "1".to_owned())]),
                label: Some("\u{f0968}  Files".to_owned()),
                command: None,
            }
        );
        assert_eq!(
            **second,
            LayoutNode::Pane {
                pane_id: None,
                cwd: Some("/projects/example".to_owned()),
                env: BTreeMap::new(),
                label: Some("\u{f489}  term".to_owned()),
                command: None,
            }
        );
    }

    /// `split_from` moves a pane's leaf down the tree, so the leaves stop matching config
    /// order — which is why the response is mapped back by traversal position, not index.
    #[test]
    fn split_from_reorders_the_leaves_away_from_config_order() {
        let mut layout = default_layout();
        layout.panes[2].split_from = Some("agent".to_owned());

        let (root, leaves) = build_layout_tree(&layout, &popup_choice(), "/projects/example");

        assert_eq!(leaves, ["agent", "term", "files"]);
        let LayoutNode::Split { first, .. } = &root else {
            panic!("expected a split at the root: {root:?}");
        };
        let LayoutNode::Split { direction, .. } = &**first else {
            panic!("expected `agent` to have been split: {first:?}");
        };
        assert_eq!(direction, "down");
    }

    #[test]
    fn default_layout_reproduces_side_pane_calls() {
        let client = FakeClient::default();
        queue_popup_splits(&client);

        build_side_panes(&client, &default_layout(), &popup_choice(), "root").unwrap();

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

        build_side_panes(&client, &layout, &popup_choice(), "root").unwrap();

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

        build_side_panes(&client, &layout, &popup_choice(), "root").unwrap();

        assert!(!client
            .calls
            .into_inner()
            .iter()
            .any(|(method, params)| { method == "pane.rename" && params["pane_id"] == "p2" }));
    }

    #[test]
    fn launch_target_defaults_to_the_first_agent_pane_and_honours_a_named_one() {
        let mut layout = default_layout();
        layout.panes[1].pane_type = PaneType::Agent;
        layout.panes[1].command = None;

        let default = launch_target(&layout, &launch_options(None, false)).unwrap();
        assert_eq!(default.name, "agent");

        // Naming a side pane is only legal when nothing is being built around it.
        let mut named = launch_options(None, true);
        named.pane = Some("files".to_owned());
        assert_eq!(launch_target(&layout, &named).unwrap().name, "files");
    }

    #[test]
    fn launch_rejects_a_layout_it_cannot_reproduce_in_the_current_pane() {
        let mut shell_root = default_layout();
        shell_root.panes[0].pane_type = PaneType::Shell;
        let error = launch_target(&shell_root, &launch_options(None, false)).unwrap_err();
        assert!(error.to_string().contains("no agent pane"), "{error}");

        // An agent pane that is not the root cannot be reached by injecting into the pane
        // the launcher is already running in.
        let mut side_agent = default_layout();
        side_agent.panes[0].pane_type = PaneType::Shell;
        side_agent.panes[1].pane_type = PaneType::Agent;
        side_agent.panes[1].command = None;
        let error = launch_target(&side_agent, &launch_options(None, false)).unwrap_err();
        assert!(error.to_string().contains("not the tab root"), "{error}");

        let mut absent = launch_options(None, true);
        absent.pane = Some("nope".to_owned());
        let error = launch_target(&default_layout(), &absent).unwrap_err();
        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn a_side_agent_pane_is_typed_into_its_split() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        // `files` becomes a second agent pane; the trailing `term` shell stays put.
        layout.panes[1].pane_type = PaneType::Agent;
        layout.panes[1].command = None;
        let mut choice = popup_choice();
        choice.agents.push(PaneAgent {
            pane: "files".to_owned(),
            launch: vec!["claude".to_owned(), "--model".to_owned(), "opus".to_owned()],
            start: None,
            kind: None,
            agent_name: "claude code".to_owned(),
            option_name: Some("Opus".to_owned()),
            effort: None,
        });
        queue_popup_splits(&client);

        build_side_panes(&client, &layout, &choice, "root").unwrap();

        let calls = client.calls.into_inner();
        let inputs = calls
            .iter()
            .filter(|(method, _)| method == "pane.send_input")
            .map(|(_, params)| (params["pane_id"].clone(), params["text"].clone()))
            .collect::<Vec<_>>();
        // Quoted argument by argument, exactly as the root agent pane is launched.
        assert_eq!(inputs, [(json!("p2"), json!("'claude' '--model' 'opus'"))]);
    }

    #[test]
    fn a_side_agent_pane_with_a_kind_starts_through_herdr() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes[1].pane_type = PaneType::Agent;
        layout.panes[1].command = None;
        let mut choice = popup_choice();
        choice.agents.push(PaneAgent {
            pane: "files".to_owned(),
            launch: vec!["claude".to_owned(), "--model".to_owned(), "opus".to_owned()],
            start: Some(AgentStart {
                kind: "claude".to_owned(),
                args: vec!["--model".to_owned(), "opus".to_owned()],
            }),
            kind: None,
            agent_name: "claude code".to_owned(),
            option_name: Some("Opus".to_owned()),
            effort: None,
        });
        queue_popup_splits(&client);

        build_side_panes(&client, &layout, &choice, "root").unwrap();

        let calls = client.calls.into_inner();
        let starts = calls
            .iter()
            .filter(|(method, _)| method == "agent.start")
            .map(|(_, params)| params.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            starts,
            [json!({
                "pane_id": "p2", "name": "files",
                "kind": "claude", "args": ["--model", "opus"],
            })]
        );
        assert!(
            !calls
                .iter()
                .any(|(method, params)| method == "pane.send_input" && params["pane_id"] == "p2"),
            "{calls:?}"
        );
    }

    #[test]
    fn an_unlabelled_agent_pane_falls_back_to_its_harness_name() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes[1].pane_type = PaneType::Agent;
        layout.panes[1].command = None;
        layout.panes[1].label = None;
        layout.panes[1].icon = None;
        let mut choice = popup_choice();
        choice.agents.push(pane_agent("files", "codex"));
        queue_popup_splits(&client);

        build_side_panes(&client, &layout, &choice, "root").unwrap();

        let renamed = client
            .calls
            .into_inner()
            .iter()
            .find(|(method, params)| method == "pane.rename" && params["pane_id"] == "p2")
            .map(|(_, params)| params["label"].clone());
        assert_eq!(renamed, Some(json!("codex")));
    }

    #[test]
    fn a_shell_root_starts_nothing_in_the_tab_root() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes[0].pane_type = PaneType::Shell;
        let choice = AgentChoice {
            agents: Vec::new(),
            ..popup_choice()
        };
        queue_popup_apply(&client);

        create_popup_tab(&client, &layout, &choice, None).unwrap();

        let calls = client.calls.into_inner();
        assert!(
            !calls.iter().any(|(method, params)| {
                method == "pane.send_input" && params["pane_id"] == "p1"
            }),
            "{calls:?}"
        );
        // The tab is still named and focused; only the harness is absent.
        assert!(calls.iter().any(|(method, _)| method == "tab.focus"));
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

        build_side_panes(&client, &layout, &popup_choice(), "root").unwrap();

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
            build_side_panes(&client, &default_layout(), &popup_choice(), "root").unwrap_err();

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
            pane: None,
            restart: false,
            resume: None,
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
    fn popup_reproduces_the_exact_four_call_sequence() {
        let client = FakeClient::default();
        queue_popup_apply(&client);

        create_popup_tab(&client, &default_layout(), &popup_choice(), None).unwrap();

        assert_eq!(
            client.calls.into_inner(),
            vec![
                (
                    "layout.apply".to_owned(),
                    json!({
                        "root": {
                            "type": "split", "direction": "right", "ratio": 0.38,
                            "first": {
                                "type": "pane",
                                "cwd": "/projects/example",
                                "env": { "Q_NO_BANNER": "1" },
                                "label": "\u{f4af}  review",
                            },
                            "second": {
                                "type": "split", "direction": "down", "ratio": 0.9,
                                "first": {
                                    "type": "pane",
                                    "cwd": "/projects/example",
                                    "env": { "Q_NO_BANNER": "1" },
                                    "label": "\u{f0968}  Files",
                                },
                                "second": {
                                    "type": "pane",
                                    "cwd": "/projects/example",
                                    "label": "\u{f489}  term",
                                },
                            },
                        },
                        "tab_label": "\u{f4af}  review",
                        "focus": false,
                    }),
                ),
                ("tab.focus".to_owned(), json!({ "tab_id": "t1" })),
                (
                    "pane.send_input".to_owned(),
                    json!({
                        "pane_id": "p1", "text": "'codex' '--profile work'", "keys": ["enter"],
                    })
                ),
                (
                    "pane.send_input".to_owned(),
                    json!({ "pane_id": "p2", "text": "yazi .", "keys": ["enter"] })
                ),
            ]
        );
    }

    /// Herdr starts the harness itself, under the name the pane already wears.
    #[test]
    fn an_agent_with_a_kind_starts_through_herdr() {
        let client = FakeClient::default();
        queue_popup_apply(&client);
        let mut choice = popup_choice();
        choice.agents[0].start = Some(AgentStart {
            kind: "codex".to_owned(),
            args: vec!["--profile work".to_owned()],
        });

        create_popup_tab(&client, &default_layout(), &choice, None).unwrap();

        let calls = client.calls.into_inner();
        let starts = calls
            .iter()
            .filter(|(method, _)| method == "agent.start")
            .map(|(_, params)| params.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            starts,
            [json!({
                "pane_id": "p1", "name": "review",
                "kind": "codex", "args": ["--profile work"],
            })]
        );
        // Nothing is typed into the agent pane any more; the command pane still is.
        let inputs = calls
            .iter()
            .filter(|(method, _)| method == "pane.send_input")
            .map(|(_, params)| params["pane_id"].clone())
            .collect::<Vec<_>>();
        assert_eq!(inputs, [json!("p2")]);
    }

    #[test]
    fn a_command_override_keeps_the_typed_argv_path() {
        // `ccr code` replaces the executable a kind implies, so `agent.start` cannot run it.
        assert!(build_start(&Config::test_default(), "claude code", Some("CCR"), None).is_none());

        let client = FakeClient::default();
        queue_popup_apply(&client);
        let mut choice = popup_choice();
        choice.agents[0] = PaneAgent {
            pane: "agent".to_owned(),
            launch: vec!["ccr".to_owned(), "code".to_owned()],
            start: None,
            kind: None,
            agent_name: "claude code".to_owned(),
            option_name: Some("CCR".to_owned()),
            effort: None,
        };

        create_popup_tab(&client, &default_layout(), &choice, None).unwrap();

        let calls = client.calls.into_inner();
        assert!(
            !calls.iter().any(|(method, _)| method == "agent.start"),
            "{calls:?}"
        );
        assert!(
            calls.iter().any(|(method, params)| {
                method == "pane.send_input" && params["text"] == "'ccr' 'code'"
            }),
            "{calls:?}"
        );
    }

    /// `agent.start` waits for the harness to be ready, so its timeout is a tab failure.
    #[test]
    fn a_timed_out_agent_start_becomes_a_flow_error() {
        let client = FakeClient::default();
        queue_popup_apply(&client);
        client.queue_error("agent.start", "timeout", "agent did not become ready");
        let mut choice = popup_choice();
        choice.agents[0].start = Some(AgentStart {
            kind: "codex".to_owned(),
            args: Vec::new(),
        });

        let error = create_popup_tab(&client, &default_layout(), &choice, None).unwrap_err();

        let flow_error = error.downcast_ref::<FlowError>().unwrap();
        assert_eq!(flow_error.title(), Some("Agent tab failed"));
        let chain = flow_error.chain();
        assert!(chain.contains("agent did not become ready"), "{chain}");
        assert!(chain.contains("pane agent"), "{chain}");
    }

    /// A pane `layout.apply` created moments ago is still loading its shell profile, so the
    /// first `agent.start` is refused and only a retry gets the tab built.
    #[test]
    fn a_pane_not_yet_at_its_prompt_is_retried_until_it_is() {
        let client = FakeClient::default();
        queue_popup_apply(&client);
        client.queue_error(
            "agent.start",
            PANE_BUSY,
            "agent target pane w1:p1 is not an available shell",
        );
        let mut choice = popup_choice();
        choice.agents[0].start = Some(AgentStart {
            kind: "codex".to_owned(),
            args: Vec::new(),
        });

        create_popup_tab(&client, &default_layout(), &choice, None).unwrap();

        let calls = client.calls.borrow();
        let starts = calls
            .iter()
            .filter(|(method, _)| method == "agent.start")
            .count();
        assert_eq!(starts, 2);
        // The refusal is spent on the retry, not reported, so no tab was torn down.
        assert!(
            !calls.iter().any(|(method, _)| method == "tab.close"),
            "{calls:?}"
        );
    }

    /// Two tabs opened under the same usage label reduce to the same agent name, which Herdr
    /// refuses, so the second start takes a numbered name instead of closing its tab.
    #[test]
    fn a_taken_agent_name_is_retried_with_a_number() {
        let client = FakeClient::default();
        queue_popup_apply(&client);
        client.queue_error(
            "agent.start",
            NAME_TAKEN,
            "agent name discuss is already used",
        );
        let mut choice = popup_choice();
        choice.agents[0].start = Some(AgentStart {
            kind: "codex".to_owned(),
            args: Vec::new(),
        });

        create_popup_tab(&client, &default_layout(), &choice, None).unwrap();

        let calls = client.calls.borrow();
        let names: Vec<String> = calls
            .iter()
            .filter(|(method, _)| method == "agent.start")
            .map(|(_, params)| params["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names.len(), 2, "{calls:?}");
        assert_eq!(names[1], format!("{}-2", names[0]));
        assert!(
            !calls.iter().any(|(method, _)| method == "tab.close"),
            "{calls:?}"
        );
    }

    #[test]
    fn a_numbered_agent_name_stays_within_herdrs_limit() {
        assert_eq!(numbered_agent_name("discuss", 2), "discuss-2");
        let long = "a".repeat(32);
        let numbered = numbered_agent_name(&long, 12);
        assert_eq!(numbered.len(), 32);
        assert!(numbered.ends_with("a-12"), "{numbered}");
    }

    /// Herdr refuses `invalid_agent_name` outright, and every real layout labels its panes
    /// with a glyph, a capital, or a space.
    #[test]
    fn a_pane_label_is_reduced_to_a_name_herdr_accepts() {
        for (label, expected) in [
            ("\u{f09d1}  main", "main"),
            ("VERIFY", "verify"),
            ("Agentic Coding", "agentic-coding"),
            ("claude code", "claude-code"),
            ("gpt-5.6_sol", "gpt-5-6_sol"),
            // Nothing usable in the label at all, so the kind stands in.
            ("\u{f09d1}", "claude"),
            ("", "claude"),
            ("繁體中文", "claude"),
        ] {
            let name = agent_name(label, "claude");
            assert_eq!(name, expected, "{label:?}");
            assert!(name.len() <= 32, "{name}");
            assert!(
                name.starts_with(|first: char| first.is_ascii_lowercase()),
                "{name}"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
                "{name}"
            );
        }

        // Thirty-three characters in, thirty-two out, with no separator left dangling.
        assert_eq!(agent_name(&"a".repeat(33), "claude").len(), 32);
        assert_eq!(
            agent_name(&format!("{}-x", "b".repeat(31)), "claude").len(),
            31
        );
    }

    #[test]
    fn build_start_carries_the_kind_with_option_and_extra_args() {
        let mut config = Config::test_default();
        config.agents[0].extra_args = vec!["--search".to_owned()];

        let start = build_start(&config, "claude code", Some("Opus"), None).unwrap();

        assert_eq!(start.kind, "claude");
        assert_eq!(start.args, ["--model", "claude-opus-4-8", "--search"]);
        // An agent whose entry names no kind has no executable Herdr can derive.
        config.agents[1].kind = None;
        assert!(build_start(&config, "codex", None, None).is_none());
    }

    /// `agent.start` derives the executable from the kind and appends only the args, so an
    /// agent that names anything else in `command` has to keep the typed-argv path.
    #[test]
    fn an_agent_command_that_is_not_the_bare_kind_keeps_the_typed_argv_path() {
        for command in [
            vec!["claude".to_owned(), "--verbose".to_owned()],
            vec!["claude-latest".to_owned()],
        ] {
            let mut config = Config::test_default();
            config.agents[0].command = command.clone();
            assert!(
                build_start(&config, "claude code", Some("Opus"), None).is_none(),
                "{command:?}"
            );
        }
    }

    /// Every agent pane the plugin does not `exec` into reports from `start_pane`, so its
    /// options come from the pane's own agent rather than the launch it never runs.
    #[test]
    fn a_reporter_is_described_for_an_agent_pane_that_names_a_kind() {
        let mut choice = popup_choice();
        assert_eq!(reporter_options(&choice.agents[0], &choice, "p1", 7), None);

        choice.agents[0].kind = Some("codex".to_owned());
        assert_eq!(
            reporter_options(&choice.agents[0], &choice, "p1", 7),
            Some(session::ReportOptions {
                pane_id: "p1".to_owned(),
                agent: "codex".to_owned(),
                kind: "codex".to_owned(),
                cwd: PathBuf::from("/projects/example"),
                since_ms: 7,
            })
        );
    }

    #[test]
    fn popup_workspace_id_is_omitted_when_empty_and_sent_when_present() {
        for (workspace, expected) in [(None, None), (Some("w1".to_owned()), Some(json!("w1")))] {
            let client = FakeClient::default();
            queue_popup_apply(&client);
            create_popup_tab(&client, &default_layout(), &popup_choice(), workspace).unwrap();
            assert_eq!(
                client.calls.borrow()[0].1.get("workspace_id"),
                expected.as_ref()
            );
        }
    }

    /// The response's leaves carry Herdr's pane ids in the order we sent ours, so the third
    /// leaf's id is what the third pane of the fold — not of the config — receives.
    #[test]
    fn a_reordered_fold_still_sends_each_command_to_its_own_pane() {
        let client = FakeClient::default();
        let mut layout = default_layout();
        layout.panes[2].split_from = Some("agent".to_owned());
        queue_popup_apply(&client);

        create_popup_tab(&client, &layout, &popup_choice(), None).unwrap();

        let inputs = client
            .calls
            .into_inner()
            .iter()
            .filter(|(method, _)| method == "pane.send_input")
            .map(|(_, params)| (params["pane_id"].clone(), params["text"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            inputs,
            [
                (json!("p1"), json!("'codex' '--profile work'")),
                (json!("p3"), json!("yazi .")),
            ]
        );
    }

    #[test]
    fn a_layout_apply_reply_that_does_not_match_the_layout_is_rejected() {
        let client = FakeClient::default();
        client.queue_response(
            "layout.apply",
            json!({
                "type": "layout_apply",
                "layout": {
                    "workspace_id": "w1", "tab_id": "t1", "zoomed": false,
                    "focused_pane_id": "p1",
                    "root": { "type": "pane", "pane_id": "p1" },
                },
            }),
        );

        let error =
            create_popup_tab(&client, &default_layout(), &popup_choice(), None).unwrap_err();

        assert!(format!("{error:#}").contains("1 pane"), "{error:#}");
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

    /// Apply is atomic, so its own failure leaves nothing on screen; every later step runs
    /// inside a tab that exists, and abandoning it would strand a tab with no agent.
    #[test]
    fn a_popup_failure_after_apply_closes_the_tab_it_built() {
        let methods = [
            "layout.apply",
            "tab.focus",
            "pane.send_input",
            "pane.send_input",
        ];
        for failure_index in 0..methods.len() {
            let client = FakeClient::default();
            for (index, method) in methods.iter().enumerate() {
                if index == failure_index {
                    client.queue_error(method, "injected", "failure");
                    break;
                }
                if *method == "layout.apply" {
                    queue_popup_apply(&client);
                } else {
                    client.queue_response(method, json!({ "type": "ok" }));
                }
            }

            let error =
                create_popup_tab(&client, &default_layout(), &popup_choice(), None).unwrap_err();
            let flow_error = error.downcast_ref::<FlowError>().unwrap();
            assert_eq!(flow_error.title(), Some("Agent tab failed"));
            assert!(flow_error.chain().contains("injected"));
            let calls = client.calls.borrow();
            let closed = calls
                .iter()
                .filter(|call| call.0 == "tab.close")
                .map(|call| call.1.clone())
                .collect::<Vec<_>>();
            if failure_index == 0 {
                assert_eq!(flow_error.prefix(), None);
                assert!(closed.is_empty(), "{calls:?}");
            } else {
                assert_eq!(flow_error.prefix(), Some("The incomplete tab was closed."));
                assert_eq!(closed, [json!({ "tab_id": "t1" })]);
            }
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
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
            ["codex"]
        );
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
            ["codex", "--dangerously-bypass-approvals-and-sandbox"]
        );
        config.agents[1].extra_args = ["--search", "--profile", "work"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
            ["codex", "--search", "--profile", "work"]
        );
        config.agents[1].extra_args = vec!["--profile work".to_owned()];
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
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
        model_rows: Vec<Vec<ModelRow>>,
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
                model_rows: Vec::new(),
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
        /// An answer names a row, optionally `row|effort`; a bare row is picked where it
        /// started, which is what enter without an arrow does.
        fn choose_model(
            &mut self,
            title: &str,
            _: &str,
            rows: &[ModelRow],
        ) -> Result<Option<(usize, Option<String>)>> {
            self.titles.push(title.to_owned());
            self.options
                .push(rows.iter().map(|row| row.label.clone()).collect());
            self.model_rows.push(rows.to_vec());
            let Some(answer) = self.answers.pop_front().flatten() else {
                return Ok(None);
            };
            let (label, effort) = match answer.split_once('|') {
                Some((label, effort)) => (label.to_owned(), Some(effort.to_owned())),
                None => (answer, None),
            };
            Ok(rows.iter().position(|row| row.label == label).map(|index| {
                let effort =
                    effort.or_else(|| rows[index].efforts.get(rows[index].effort).cloned());
                (index, effort)
            }))
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

    /// `bare_layout` plus a second agent pane split off it.
    fn two_agent_layout(name: &str) -> TabLayout {
        let mut layout = bare_layout(name);
        let mut reviewer = layout.panes[0].clone();
        reviewer.name = "reviewer".to_owned();
        reviewer.label = Some("Reviewer".to_owned());
        reviewer.direction = Some(crate::config::Direction::Right);
        reviewer.ratio = Some(0.5);
        layout.panes.push(reviewer);
        layout
    }

    #[test]
    fn each_agent_pane_runs_its_own_harness_and_model_menu_in_layout_order() {
        let config = config();
        let layout = two_agent_layout("pair");
        let mut menu = FakeMenu::new([
            Some(TEST_CLAUDE_LABEL),
            Some("Opus"),
            Some(TEST_CODEX_LABEL),
            Some(USAGE_DISCUSS),
        ]);

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

        assert!(menu.answered_everything());
        // codex has no options, so it contributes no model menu. The usage menu still runs
        // exactly once, after every pane is decided.
        assert_eq!(
            menu.titles,
            [
                format!("{HARNESS_TITLE} · agent"),
                format!("{TEST_CLAUDE_LABEL} · agent"),
                format!("{HARNESS_TITLE} · Reviewer"),
                USAGE_TITLE.to_owned(),
            ]
        );
        let agents = choice
            .agents
            .iter()
            .map(|agent| (agent.pane.as_str(), agent.agent_name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(agents, [("agent", "claude code"), ("reviewer", "codex")]);
    }

    #[test]
    fn a_single_agent_pane_leaves_the_menu_titles_unqualified() {
        let config = config();
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), Some(USAGE_DISCUSS)]);

        choose_agent_with(
            &config,
            &bare_layout("solo"),
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.titles, [HARNESS_TITLE, USAGE_TITLE]);
    }

    #[test]
    fn cancelling_the_second_agent_pane_cancels_the_whole_flow() {
        let config = config();
        let layout = two_agent_layout("pair");
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), None]);

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

    #[test]
    fn a_pinned_pane_beside_an_asked_one_runs_only_the_asked_menus() {
        let config = config();
        let mut layout = two_agent_layout("pair");
        layout.panes[1].agent = Some("claude code".to_owned());
        layout.panes[1].option_name = Some("Opus".to_owned());
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), Some(USAGE_DISCUSS)]);

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

        assert_eq!(
            menu.titles,
            [format!("{HARNESS_TITLE} · agent"), USAGE_TITLE.to_owned()]
        );
        assert_eq!(choice.agents[1].agent_name, "claude code");
        assert_eq!(choice.agents[1].option_name.as_deref(), Some("Opus"));
    }

    /// `bare_layout` with its one pane turned into a plain shell.
    fn shell_only_layout(name: &str, label: &str) -> TabLayout {
        let mut layout = bare_layout(name);
        layout.label = Some(label.to_owned());
        layout.panes[0].pane_type = PaneType::Shell;
        layout
    }

    #[test]
    fn a_layout_with_no_agent_pane_runs_no_harness_menu_and_asks_for_a_name() {
        let config = config();
        let mut menu = FakeMenu::new([Some("scratch")]);

        let choice = choose_agent_with(
            &config,
            &shell_only_layout("blank", "Blank Tab"),
            Path::new("/project"),
            false,
            None,
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert!(menu.answered_everything());
        assert!(choice.agents.is_empty());
        assert_eq!(choice.label, "scratch");
    }

    #[test]
    fn an_empty_tab_name_keeps_the_layout_label_but_escape_cancels() {
        let config = config();
        let layout = shell_only_layout("blank", "Blank Tab");

        // gum exits zero with empty stdout when the field is submitted blank, and non-zero
        // when it is escaped. Only the second is a cancellation.
        let mut blank = FakeMenu::new([Some("   ")]);
        let choice = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut blank,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(choice.label, "Blank Tab");

        let mut escaped = FakeMenu::new([None]);
        let cancelled = choose_agent_with(
            &config,
            &layout,
            Path::new("/project"),
            false,
            None,
            &mut escaped,
            &FakeGit::nowhere(),
        )
        .unwrap();
        assert_eq!(cancelled, None);
    }

    #[test]
    fn the_menu_rows_ellipsis_does_not_reach_the_tab_name() {
        let config = config();
        let layout = shell_only_layout("blank-tab", "Blank Tab\u{2026}");
        let mut menu = FakeMenu::new([Some("")]);

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

        assert_eq!(choice.label, "Blank Tab");
    }

    #[test]
    fn a_pinned_tab_label_skips_the_name_prompt() {
        let config = config();
        let mut layout = shell_only_layout("blank", "Blank Tab");
        layout.tab_label = Some("Notes".to_owned());
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

        assert_eq!(choice.label, "Notes");
    }

    #[test]
    fn a_stored_choice_is_not_replayed_when_several_panes_are_asked_about() {
        let config = config();
        let layout = two_agent_layout("pair");
        let panes = layout
            .agent_panes()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>();
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL), Some(TEST_CODEX_LABEL)]);

        choose_agent_with_last(
            &config,
            &layout,
            &panes,
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Opus".to_owned()),
                effort: None,
                layout: "agentic-coding".to_owned(),
                pane: "agent".to_owned(),
                session: None,
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        // The record names one pane, so with two being asked it cannot say which. Every
        // harness menu therefore opens on the plain agent list.
        assert!(
            !menu.options[0][0].starts_with(USE_LAST_PREFIX),
            "{:?}",
            menu.options[0]
        );
        assert!(
            !menu.options[1][0].starts_with(USE_LAST_PREFIX),
            "{:?}",
            menu.options[1]
        );
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
        assert_eq!(choice.agents[0].agent_name, "claude code");
        assert_eq!(choice.agents[0].option_name.as_deref(), Some("Opus"));
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
        assert_eq!(choice.agents[0].option_name, None);
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
    fn each_model_row_starts_on_its_default_effort_and_the_pick_reaches_the_argv() {
        let config = config();
        let layout = bare_layout("ask");
        let mut menu = FakeMenu::new([
            Some(TEST_CLAUDE_LABEL),
            Some("OpusPlan (Sonnet)|xhigh"),
            Some(USAGE_DISCUSS),
        ]);

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

        let rows = &menu.model_rows[0];
        assert_eq!(rows[1].efforts[rows[1].effort], "medium");
        // An option without a default effort offers none to step through.
        assert!(rows[0].efforts.is_empty());
        let agent = &choice.agents[0];
        assert_eq!(agent.effort.as_deref(), Some("xhigh"));
        assert_eq!(
            agent.launch,
            [
                "claude",
                "--model",
                "opusplan",
                "--effort",
                "xhigh",
                "argument with space"
            ]
        );
    }

    #[test]
    fn use_last_replays_the_stored_effort() {
        let mut config = config();
        config.tab_layouts.push(default_layout());
        let entry = format!("{USE_LAST_PREFIX}{TEST_CLAUDE_LABEL} · OpusPlan (Sonnet) · max");
        let mut menu = FakeMenu::new([Some(entry.as_str())]);

        let layout = default_layout();
        let panes = layout
            .agent_panes()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>();
        let choice = choose_agent_with_last(
            &config,
            &layout,
            &panes,
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("OpusPlan (Sonnet)".to_owned()),
                effort: Some("max".to_owned()),
                layout: "agentic-coding".to_owned(),
                pane: "agent".to_owned(),
                session: None,
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options[0][0], entry);
        assert_eq!(choice.agents[0].effort.as_deref(), Some("max"));
        assert_eq!(choice.agents[0].launch[3..5], ["--effort", "max"]);
    }

    #[test]
    fn use_last_is_first_and_skips_model_and_usage_menus() {
        let mut config = config();
        config.tab_layouts.push(default_layout());
        let entry = format!("{USE_LAST_PREFIX}{TEST_CLAUDE_LABEL} · Opus");
        let mut menu = FakeMenu::new([Some(entry.as_str())]);

        let layout = default_layout();
        let panes = layout
            .agent_panes()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>();
        let choice = choose_agent_with_last(
            &config,
            &layout,
            &panes,
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Opus".to_owned()),
                effort: None,
                layout: "agentic-coding".to_owned(),
                pane: "agent".to_owned(),
                session: None,
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options.len(), 1);
        assert_eq!(menu.options[0][0], entry);
        assert_eq!(choice.agents[0].agent_name, "claude code");
        assert_eq!(choice.agents[0].option_name.as_deref(), Some("Opus"));
    }

    #[test]
    fn stale_last_choice_does_not_add_a_menu_entry() {
        let config = config();
        let mut menu = FakeMenu::new([Some(TEST_CODEX_LABEL)]);

        let layout = default_layout();
        let panes = layout
            .agent_panes()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>();
        let choice = choose_agent_with_last(
            &config,
            &layout,
            &panes,
            Path::new("/project"),
            false,
            Some("review"),
            Some(state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Removed".to_owned()),
                effort: None,
                layout: "agentic-coding".to_owned(),
                pane: "agent".to_owned(),
                session: None,
                recorded_at: 1,
            }),
            &mut menu,
            &FakeGit::nowhere(),
        )
        .unwrap()
        .unwrap();

        assert_eq!(menu.options[0][0], TEST_CLAUDE_LABEL);
        assert_eq!(choice.agents[0].agent_name, "codex");
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
    fn resume_arguments_sit_between_the_command_and_the_option_args() {
        let mut config = Config::test_default();
        config.agents[1].extra_args = vec!["--search".to_owned()];

        // codex's `resume` is a subcommand, so it has to follow the executable directly
        // and precede the model args; claude's flag form lands in the same slot.
        assert_eq!(
            build_launch(&config, "codex", None, None, Some("019-abc")).unwrap(),
            ["codex", "resume", "019-abc", "--search"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), None, Some("uuid-1")).unwrap(),
            ["claude", "--resume", "uuid-1", "--model", "claude-opus-4-8"]
        );
        // A command override still resumes: `ccr code` writes a claude transcript.
        assert_eq!(
            build_launch(&config, "claude code", Some("CCR"), None, Some("uuid-1")).unwrap(),
            ["ccr", "code", "--resume", "uuid-1"]
        );

        // An agent Herdr has no kind for gets no resume arguments at all.
        config.agents[1].kind = None;
        assert_eq!(
            build_launch(&config, "codex", None, None, Some("019-abc")).unwrap(),
            ["codex", "--search"]
        );
    }

    #[test]
    fn launch_commands_match_every_harness_and_model_rule() {
        let config = Config::test_default();
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
            ["codex"]
        );
        assert_eq!(
            build_launch(&config, "opencode", None, None, None).unwrap(),
            ["opencode"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("CCR"), None, None).unwrap(),
            ["ccr", "code"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), None, None).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(
            build_launch(
                &config,
                "claude code",
                Some("OpusPlan (Sonnet)"),
                None,
                None
            )
            .unwrap(),
            ["claude", "--model", "opusplan", "--effort", "medium"]
        );
        assert_eq!(
            build_launch(&config, "claude code", Some("Fable 5"), None, None).unwrap(),
            ["claude", "--model", "claude-fable-5"]
        );
    }

    #[test]
    fn a_chosen_effort_replaces_the_option_default_before_the_extra_args() {
        let mut config = Config::test_default();
        config.agents[0].extra_args = vec!["--search".to_owned()];
        let plan = Some("OpusPlan (Sonnet)");

        assert_eq!(
            build_launch(&config, "claude code", plan, Some("xhigh"), None).unwrap(),
            ["claude", "--model", "opusplan", "--effort", "xhigh", "--search"]
        );
        assert_eq!(
            build_start(&config, "claude code", plan, Some("xhigh"))
                .unwrap()
                .args,
            ["--model", "opusplan", "--effort", "xhigh", "--search"]
        );
        // A level the config no longer lists falls back to the option's own default, so a
        // stored choice outlives an edited efforts list.
        assert_eq!(
            build_launch(&config, "claude code", plan, Some("gone"), None).unwrap(),
            ["claude", "--model", "opusplan", "--effort", "medium", "--search"]
        );
        // An option without a default takes no effort, whatever is asked for.
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), Some("high"), None).unwrap(),
            ["claude", "--model", "claude-opus-4-8", "--search"]
        );
    }

    #[test]
    fn extra_args_reach_plain_and_command_override_options() {
        let mut config = config();
        config.agents[0].extra_args = ["--search", "--profile", "work"]
            .map(str::to_owned)
            .to_vec();

        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), None, None).unwrap(),
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
            build_launch(&config, "claude code", Some("CCR"), None, None).unwrap(),
            ["ccr", "code", "--search", "--profile", "work"]
        );
    }

    #[test]
    fn command_override_still_takes_its_option_args() {
        let mut config = config();
        config.agents[0].extra_args.clear();
        config.agents[0].options[2].args = vec!["--flag".to_owned()];

        assert_eq!(
            build_launch(&config, "claude code", Some("CCR"), None, None).unwrap(),
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
            build_launch(&config, "claude code", Some("Opus"), None, None).unwrap(),
            ["claude", "code", "--cd", "/Users/q/My Projects"]
        );
    }

    #[test]
    fn missing_launch_names_are_named_errors() {
        let config = config();

        let error = build_launch(&config, "missing agent", None, None, None).unwrap_err();
        assert!(error.to_string().contains("missing agent"));
        let error =
            build_launch(&config, "claude code", Some("missing option"), None, None).unwrap_err();
        assert!(error.to_string().contains("missing option"));
        let agent_name = "claude code";
        let error = build_launch(&config, agent_name, None, None, None).unwrap_err();
        assert!(error.to_string().contains(agent_name));
    }

    #[test]
    fn bypass_flags_are_absent_unless_configured() {
        let mut config = config();
        config.agents[0].extra_args = Vec::new();
        config.agents[1].extra_args = Vec::new();
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), None, None).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
            ["codex"]
        );

        config.agents[0].extra_args = vec!["--dangerously-skip-permissions".to_owned()];
        config.agents[1].extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
        assert_eq!(
            build_launch(&config, "claude code", Some("Opus"), None, None).unwrap(),
            [
                "claude",
                "--model",
                "claude-opus-4-8",
                "--dangerously-skip-permissions"
            ]
        );
        assert_eq!(
            build_launch(&config, "codex", None, None, None).unwrap(),
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
            agents: vec![pane_agent("agent", "codex")],
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
        assert_eq!(choice.agents[0].launch, ["opencode"]);
        assert_eq!(choice.agents[0].agent_name, "opencode");
        assert_eq!(choice.agents[0].option_name, None);
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
        assert_eq!(choice.agents[0].option_name.as_deref(), Some("Opus"));
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
                model_rows: Vec::new(),
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
                model_rows: Vec::new(),
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
