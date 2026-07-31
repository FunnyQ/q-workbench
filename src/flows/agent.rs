use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config::Config;

const HARNESS_TITLE: &str = "\u{f169f}  Launch Agent";
const HARNESS_CLAUDE: &str = "\u{f15ce}  claude code";
const HARNESS_CODEX: &str = "\u{ee0d}  codex";
const HARNESS_OPENCODE: &str = "\u{f169f}  opencode";
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
) -> Result<Option<AgentChoice>> {
    let mut menu = GumMenu::new(cols, lines);
    choose_agent_with(config, cwd, worktree, fixed_usage, &mut menu, &RealGit)
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
    ChoiceMargin,
    None,
}

fn choose_agent_with(
    config: &Config,
    cwd: &Path,
    worktree: bool,
    fixed_usage: Option<&str>,
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

    let harness_options = [
        HARNESS_CLAUDE.to_owned(),
        HARNESS_CODEX.to_owned(),
        HARNESS_OPENCODE.to_owned(),
    ];
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
    let model_label = if harness.contains("claude code") {
        let Some(model_label) = menu.choose(MODEL_TITLE, "Choose a model.", &config.order, 6)?
        else {
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
        let mut launch = vec!["codex".to_owned()];
        launch.extend(config.codex_extra_args.clone());
        return Ok(launch);
    }
    if harness.contains("opencode") {
        return Ok(vec!["opencode".to_owned()]);
    }

    let label = model_label.context("claude harness requires a model label")?;
    let model = config
        .models
        .get(label)
        .with_context(|| format!("model label has no model entry: {label}"))?;
    // CCR is not a model: it dispatches to its own binary, with no model flag and none
    // of the claude extra args.
    if model == "CCR" {
        return Ok(vec!["ccr".to_owned(), "code".to_owned()]);
    }

    let mut launch = vec!["claude".to_owned(), "--model".to_owned(), model.clone()];
    if let Some(args) = config.model_args.get(label) {
        launch.extend(args.clone());
    }
    launch.extend(config.claude_extra_args.clone());
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
            InputIndent::ChoiceMargin,
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
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
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
    fn content_margin(&self) -> u16 {
        self.cols
            .saturating_sub(self.content_width())
            .saturating_sub(2)
            / 2
    }

    /// The left margin that centers a menu option or an input field. Menu items are
    /// treated as 24 columns wide, which is what the zsh version assumes.
    fn choice_margin(&self) -> u16 {
        self.cols.saturating_sub(24) / 2
    }

    /// Draw the centered banner: title, blank line, dim subtitle.
    ///
    /// The banner is printed line by line at a computed margin instead of being handed to
    /// another `gum` call. Wrapping an already-styled multiline banner in a second
    /// `gum style` offsets its border lines, because the outer call measures the ANSI
    /// escapes as visible width and pads each line differently — the box comes out
    /// ragged. Printing the lines here keeps the border square.
    fn render_banner(&self, title: &str, subtitle: &str) -> Result<()> {
        if io::stdout().is_terminal() {
            print!("\x1b[2J\x1b[H");
        }
        let vertical_padding = self.lines.saturating_sub(14) / 2;
        print!("{}", "\n".repeat(vertical_padding.into()));
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
        let margin = usize::from(self.content_margin());
        for line in banner.lines() {
            println!("{:margin$}{line}", "");
        }
        println!();
        io::stdout().flush().context("failed to draw menu banner")
    }

    fn padded(&self, options: &[String]) -> Vec<String> {
        let pad = " ".repeat(usize::from(self.choice_margin()));
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
        self.render_banner(title, subtitle)?;
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
        self.render_banner(title, subtitle)?;
        // --no-strict returns the typed text when it matches no branch, so the same field
        // both picks an existing branch and names a new one.
        gum_with_input(
            &[
                "filter",
                "--no-strict",
                "--height",
                "12",
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
        self.render_banner(title, subtitle)?;
        if indent == InputIndent::ChoiceMargin {
            print!("{}", " ".repeat(usize::from(self.choice_margin())));
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
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn config() -> Config {
        Config {
            dashboard_workspace: String::new(),
            claude_extra_args: vec!["argument with space".to_owned()],
            codex_extra_args: vec!["--search".to_owned()],
            project_registry_file: String::new(),
            projects_root: String::new(),
            ssh_registry_file: String::new(),
            ssh_config_file: String::new(),
            ssh_history_file: String::new(),
            order: ["Opus", "OpusPlan (Sonnet)", "CCR", "Fable 5"]
                .map(str::to_owned)
                .to_vec(),
            models: [
                ("Opus", "claude-opus-4-8"),
                ("OpusPlan (Sonnet)", "opusplan"),
                ("CCR", "CCR"),
                ("Fable 5", "claude-fable-5"),
            ]
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .into(),
            model_args: [(
                "OpusPlan (Sonnet)".to_owned(),
                vec!["--effort".to_owned(), "medium".to_owned()],
            )]
            .into(),
        }
    }

    /// Replays scripted answers in menu order, so a test can cancel at an exact step.
    struct FakeMenu {
        answers: VecDeque<Option<String>>,
    }

    impl FakeMenu {
        fn new<const N: usize>(answers: [Option<&str>; N]) -> Self {
            Self {
                answers: answers
                    .into_iter()
                    .map(|answer| answer.map(str::to_owned))
                    .collect(),
            }
        }

        fn answered_everything(&self) -> bool {
            self.answers.is_empty()
        }
    }

    impl Menu for FakeMenu {
        fn choose(&mut self, _: &str, _: &str, _: &[String], _: u8) -> Result<Option<String>> {
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
        // CCR takes neither a model flag nor the claude extra args.
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
        config.claude_extra_args = vec![
            "--add-dir".to_owned(),
            "/Users/q/My Projects".to_owned(),
            "--dangerously-skip-permissions".to_owned(),
        ];
        config.codex_extra_args = vec!["--cd".to_owned(), "/Users/q/My Projects".to_owned()];

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
        config.claude_extra_args = Vec::new();
        config.codex_extra_args = Vec::new();
        assert_eq!(
            build_launch(&config, HARNESS_CLAUDE, Some("Opus")).unwrap(),
            ["claude", "--model", "claude-opus-4-8"]
        );
        assert_eq!(
            build_launch(&config, HARNESS_CODEX, None).unwrap(),
            ["codex"]
        );

        config.claude_extra_args = vec!["--dangerously-skip-permissions".to_owned()];
        config.codex_extra_args = vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
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
        assert_eq!(menu.content_margin(), 17);
        assert_eq!(menu.choice_margin(), 28);
        assert_eq!(
            menu.padded(&[USAGE_DEBUG.to_owned()]),
            [format!("{}{USAGE_DEBUG}", " ".repeat(28))]
        );

        // A viewport too narrow for the banner narrows the box and floors both margins.
        let menu = GumMenu::new(20, 8);
        assert_eq!(menu.content_width(), 16);
        assert_eq!(menu.content_margin(), 1);
        assert_eq!(menu.choice_margin(), 0);
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
