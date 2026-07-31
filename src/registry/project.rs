//! Project registry discovery, storage, and non-interactive operations.
//!
//! This layer reproduces the discovery half of `scripts/project-registry.zsh` and
//! owns registry reads, atomic writes, source merges, `update`, and `use`.
//!
//! Every function here is deliberately infallible. The zsh original ran `find` and
//! `rg` with stderr redirected to `/dev/null`, so an unreadable directory or a
//! truncated transcript skipped that entry and discovery carried on. Returning
//! `Result` and letting the caller `unwrap_or_default()` would instead collapse a
//! whole source to empty the first time one file failed to read.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{format_description, OffsetDateTime};

const REVIEW_HEADER: &str = "Review projects (space: toggle · enter: save)";

/// Directory names `find` prunes during the `.git` sweep. Pruning happens before
/// descending: a full `node_modules` tree dwarfs the projects root itself, so
/// walking it and filtering afterwards would dominate the runtime.
const PRUNED_DIRECTORIES: &[&str] = &[
    "node_modules",
    "vendor",
    "tmp",
    "log",
    "coverage",
    "dist",
    "build",
    ".nuxt",
    ".next",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Claude,
    Codex,
    Filesystem,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Filesystem => "filesystem",
        }
    }
}

/// Resolve a candidate path to the project it belongs to, or nothing.
///
/// Yielding nothing is the normal case, not a fault: sessions outlive the
/// directories they ran in, so a stale path pointing at a deleted or temporary
/// directory is expected and must simply be dropped.
pub fn canonical_project(path: &Path, projects_root: &Path) -> Option<PathBuf> {
    if !path.is_dir() || path == Path::new("/") {
        return None;
    }

    // A session can be recorded in a directory that is not a git checkout at all
    // (`~/.claude/projects/-Users-funnyq--config` is a real example). The zsh source
    // falls back to the path itself on a failed `rev-parse`; dropping those entries
    // would silently lose every non-git project.
    let root = git_toplevel(path).unwrap_or_else(|| path.to_owned());
    if root == Path::new("/") {
        return None;
    }

    // Resolve symlinks so two paths to the same project cannot register twice.
    let root = fs::canonicalize(&root).ok()?;
    if root == Path::new("/") {
        return None;
    }

    if is_temporary_path(&root) {
        // The exception exists for a projects root that is itself a symlink into a
        // temp-like path. Without it, such a developer would have every project
        // silently dropped by the temp filter.
        let projects_root =
            fs::canonicalize(projects_root).unwrap_or_else(|_| projects_root.to_owned());
        // A strict descendant, matching the zsh glob `"$physical_projects_root"/*`.
        if !root.starts_with(&projects_root) || root == projects_root {
            return None;
        }
    }

    Some(root)
}

/// Candidate paths from Claude Code: `.entries[].projectPath` of every
/// `sessions-index.json` under `~/.claude/projects`, plus the `cwd` scraped from
/// every transcript. The index files live one level down, inside each encoded
/// project directory, so the search has to be recursive.
pub fn discover_claude_projects(home: &Path) -> Vec<(PathBuf, Source)> {
    let projects = home.join(".claude/projects");
    if !projects.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();

    for index in collect_files(&projects, &|path| file_name(path) == "sessions-index.json") {
        let Ok(contents) = fs::read_to_string(&index) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&contents) else {
            continue;
        };
        let Some(entries) = value.get("entries").and_then(Value::as_array) else {
            continue;
        };
        results.extend(entries.iter().filter_map(|entry| {
            entry
                .get("projectPath")
                .and_then(Value::as_str)
                .map(|path| (PathBuf::from(path), Source::Claude))
        }));
    }

    for transcript in collect_files(&projects, &|path| extension(path) == "jsonl") {
        if let Some(cwd) = first_transcript_cwd(&transcript) {
            results.push((PathBuf::from(cwd), Source::Claude));
        }
    }

    results
}

/// Candidate paths from Codex: the first line of every `rollout-*.jsonl` under
/// `~/.codex/sessions`. Only the first line is read because the session header
/// carries the `cwd` and a rollout can be very large.
pub fn discover_codex_projects(home: &Path) -> Vec<(PathBuf, Source)> {
    let sessions = home.join(".codex/sessions");
    if !sessions.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();
    for rollout in collect_files(&sessions, &|path| {
        let name = file_name(path);
        name.starts_with("rollout-") && name.ends_with(".jsonl")
    }) {
        let Some(line) = first_line(&rollout) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Two shapes exist: the current `session_meta` envelope and older rollouts
        // that carried `cwd` at the top level.
        let cwd = if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            value
                .pointer("/payload/cwd")
                .and_then(Value::as_str)
                .or_else(|| value.get("cwd").and_then(Value::as_str))
        } else {
            value.get("cwd").and_then(Value::as_str)
        };
        if let Some(cwd) = cwd {
            results.push((PathBuf::from(cwd), Source::Codex));
        }
    }

    results
}

/// Candidate paths from the filesystem: a `.git` sweep of the projects root.
pub fn discover_filesystem_projects(projects_root: &Path) -> Vec<(PathBuf, Source)> {
    if !projects_root.is_dir() {
        return Vec::new();
    }

    let mut results = Vec::new();
    sweep_for_git(projects_root, &mut results);
    results.sort();
    results
}

fn git_toplevel(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim_end_matches('\n');
    if root.is_empty() {
        return None;
    }
    Some(PathBuf::from(root))
}

fn sweep_for_git(directory: &Path, results: &mut Vec<(PathBuf, Source)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        // `.git` is a directory in a normal checkout but a file in a linked git
        // worktree. `find -name .git -prune -print` matched both, so both count.
        if name == ".git" {
            results.push((directory.to_owned(), Source::Filesystem));
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // `file_type` does not follow symlinks, matching `find` without `-L`.
        if !file_type.is_dir() || PRUNED_DIRECTORIES.iter().any(|pruned| name == *pruned) {
            continue;
        }
        sweep_for_git(&entry.path(), results);
    }
}

fn collect_files(directory: &Path, matches: &impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_into(directory, matches, &mut files);
    // Sorted so discovery output does not depend on directory iteration order.
    files.sort();
    files
}

fn collect_files_into(
    directory: &Path,
    matches: &impl Fn(&Path) -> bool,
    files: &mut Vec<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_files_into(&path, matches, files);
        } else if file_type.is_file() && matches(&path) {
            files.push(path);
        }
    }
}

/// The first `cwd` in a transcript, scanning line by line.
///
/// The zsh source used `rg -m1` with a regex over the escaped JSON string instead of
/// parsing the file: a transcript holds every message of a session and can reach tens
/// of megabytes, so parsing one to read a field that is identical on every line was
/// far too slow. Stopping at the first hit per file is the same trade.
fn first_transcript_cwd(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            // A transcript still being written can end mid-character; skip it rather
            // than abandoning the whole source.
            return None;
        };
        if let Some(cwd) = extract_cwd(&line) {
            return Some(cwd);
        }
    }
    None
}

/// Read the value of the first `"cwd":"…"` in a line, honouring JSON escapes.
///
/// This mirrors the `rg -o '"cwd":"([^"\\]|\\.)*"'` match, which found the key at any
/// depth, so a `cwd` nested inside an envelope is still picked up.
fn extract_cwd(line: &str) -> Option<String> {
    const KEY: &str = "\"cwd\":";
    let start = line.find("\"cwd\":\"")? + KEY.len();
    let bytes = line.as_bytes();
    // Walk from the byte after the opening quote to the first unescaped quote. Every
    // byte inspected is ASCII, so this never lands inside a multi-byte character.
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return serde_json::from_str::<String>(&line[start..=index]).ok(),
            _ => index += 1,
        }
    }
    None
}

fn first_line(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    BufReader::new(file).lines().next()?.ok()
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn extension(path: &Path) -> &str {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

/// Paths the temp filter drops: `/tmp`, `/private/tmp`, `/var/folders/*/*/T` and
/// `/private/var/folders/*/*/T`, each including their contents.
///
/// Agents run in scratch directories, and macOS puts `TMPDIR` under
/// `/var/folders/<x>/<y>/T`. Those sessions are real but their directories are
/// disposable, so registering them would fill the picker with paths that vanish.
fn is_temporary_path(path: &Path) -> bool {
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();

    matches!(
        components.as_slice(),
        ["tmp", ..]
            | ["private", "tmp", ..]
            | ["var", "folders", _, _, "T", ..]
            | ["private", "var", "folders", _, _, "T", ..]
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub version: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub generated_at: String,
    pub projects: BTreeMap<String, ProjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<u64>,
}

pub trait Clock {
    fn unix_seconds(&self) -> u64;
}

// The clock is injectable because unit tests pin exact timestamps, while parity
// comparisons must normalise `generated_at` produced by two separate processes.
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_seconds(&self) -> u64 {
        UNIX_EPOCH
            .elapsed()
            .expect("system clock is before UNIX_EPOCH")
            .as_secs()
    }
}

pub fn merge_registry(
    mut existing: ProjectRegistry,
    discovered: &BTreeMap<String, Vec<String>>,
    generated_at: String,
) -> ProjectRegistry {
    existing.generated_at = generated_at;
    for (path, entry) in &mut existing.projects {
        let mut sources = discovered
            .get(path)
            .into_iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        if entry.sources.iter().any(|source| source == "manual") {
            sources.insert("manual".to_owned());
        }
        entry.sources = sources.into_iter().collect();
    }
    existing
}

pub fn discovered_projects(home: &Path, projects_root: &Path) -> BTreeMap<String, Vec<String>> {
    let mut projects = BTreeMap::<String, BTreeSet<String>>::new();
    let records = discover_claude_projects(home)
        .into_iter()
        .chain(discover_codex_projects(home))
        .chain(discover_filesystem_projects(projects_root));

    for (candidate, source) in records {
        if let Some(project) = canonical_project(&candidate, projects_root) {
            projects
                .entry(project.to_string_lossy().into_owned())
                .or_default()
                .insert(source.as_str().to_owned());
        }
    }

    projects
        .into_iter()
        .map(|(path, sources)| (path, sources.into_iter().collect()))
        .collect()
}

pub fn registry_timestamp(clock: &impl Clock) -> Result<String> {
    let timestamp = i64::try_from(clock.unix_seconds()).context("timestamp exceeds i64")?;
    let date = OffsetDateTime::from_unix_timestamp(timestamp).context("invalid timestamp")?;
    // Build the format by hand because RFC 3339 helpers can emit subseconds and
    // numeric offsets, while jq's contract is exactly `%Y-%m-%dT%H:%M:%SZ`.
    let format = format_description::parse_borrowed::<2>(
        "[year]-[month padding:zero]-[day padding:zero]T[hour padding:zero]:[minute padding:zero]:[second padding:zero]Z",
    )
    .context("invalid registry timestamp format")?;
    date.format(&format)
        .context("failed to format registry timestamp")
}

pub fn read_registry(path: &Path) -> Result<ProjectRegistry> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let registry: ProjectRegistry = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    anyhow::ensure!(registry.version == 1, "unsupported registry version");
    Ok(registry)
}

pub fn write_registry(path: &Path, registry: &ProjectRegistry) -> Result<()> {
    write_json_atomically(path, registry)
}

/// Split the comma-separated alias field: trim each, drop empties, deduplicate.
///
/// Order is preserved deliberately. The aliases are typed by hand and the picker shows
/// them in the order given, so sorting them would reorder the user's own list under
/// them. The `BTreeSet` is a membership test only, never the output order.
pub fn normalize_aliases(input: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    input
        .split(',')
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .filter(|alias| seen.insert((*alias).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// The path of a selected menu row: the **last** tab-separated field.
///
/// Read from the end, never the start. The display column is free text and a project
/// name may contain a tab; taking field two would then yield a fragment of the name and
/// silently drop that project from the write.
pub fn parse_selection_path(line: &str) -> String {
    line.rsplit('\t').next().unwrap_or_default().to_owned()
}

/// One review row: `<marker> <name>\t<path>`, or `<name>\t<path>` when unmarked.
pub fn format_candidate_row(name: &str, path: &str, marker: &str) -> String {
    if marker.is_empty() {
        format!("{name}\t{path}")
    } else {
        format!("{marker} {name}\t{path}")
    }
}

#[derive(Clone)]
struct Candidate {
    path: String,
    row: String,
}

/// Build the review menu: one row per path, from discovery unioned with the registry.
///
/// The display column is always the path's **basename**, never the entry's stored
/// display name. `scripts/project-registry.zsh:278` computes it as
/// `$path | split("/") | last` for every row, including registered ones, so a project
/// renamed through `edit` still reviews under its basename. The stored name survives
/// the write regardless — `scan_with` clones the existing entry rather than rebuilding
/// it — so showing the basename here costs nothing and keeps the menu aligned with the
/// paths it is really about.
///
/// Row order is by path. `BTreeMap` iterates its `String` keys in byte order, which is
/// the same order jq's `keys[]` produces, so the two versions list identically.
fn candidate_rows(
    existing: &ProjectRegistry,
    discovered: &BTreeMap<String, Vec<String>>,
    rescan: bool,
) -> Vec<Candidate> {
    let mut markers = BTreeMap::<&String, &str>::new();
    for path in discovered.keys() {
        // `scan` never marks: it starts from no registry, so every row is new and a
        // marker on all of them would carry no information.
        let marker = if rescan && !existing.projects.contains_key(path) {
            "[new]"
        } else {
            ""
        };
        markers.insert(path, marker);
    }
    if rescan {
        // A registered path that discovery no longer finds. It stays on the menu so the
        // user can decide, because discovery misses are routine — a project on an
        // unmounted volume is missing, not deleted.
        for path in existing.projects.keys() {
            markers.entry(path).or_insert("[missing]");
        }
    }

    markers
        .into_iter()
        .map(|(path, marker)| Candidate {
            row: format_candidate_row(file_name(Path::new(path)), path, marker),
            path: path.clone(),
        })
        .collect()
}

/// The interactive surface of `scan`, `rescan` and `edit`, behind one trait so the
/// review logic can be tested without a TTY.
///
/// `Ok(None)` means the user cancelled — `gum` exits non-zero on escape. It is not an
/// error to report; the caller turns it into the parity contract's "registry not
/// written" message.
trait PromptRunner {
    fn choose_projects(&mut self, rows: &[String]) -> Result<Option<String>>;
    fn input(&mut self, header: &str, value: &str) -> Result<Option<String>>;
    fn visibility(&mut self, current: &str) -> Result<Option<String>>;
    fn clear(&mut self) -> Result<()>;
}

/// The review menu's `gum` arguments, kept in one place so a parity test can compare
/// them with the ones `scripts/project-registry.zsh:286-288` passes.
fn review_menu_args() -> Vec<String> {
    [
        "choose",
        "--no-limit",
        "--selected=*",
        "--ordered",
        "--height=24",
        "--no-strip-ansi",
        "--header",
        REVIEW_HEADER,
    ]
    .iter()
    .map(|argument| (*argument).to_owned())
    .collect()
}

struct GumPrompt;

impl GumPrompt {
    fn output(command: &mut Command) -> Result<Option<String>> {
        let output = command.output().context("failed to run gum")?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8(output.stdout)
                .context("gum returned invalid UTF-8")?
                .trim_end_matches('\n')
                .to_owned(),
        ))
    }
}

impl PromptRunner for GumPrompt {
    /// The review menu. Every flag is load-bearing:
    ///
    /// - `--selected=*` preselects every row, which makes "press enter" mean "keep what
    ///   you already have". Reviewing a hundred projects by selecting them one at a
    ///   time would be unusable, and the common answer is "all of them".
    /// - `--no-strip-ansi` keeps the `[new]` / `[missing]` markers intact. `gum` strips
    ///   escape sequences from its input by default and would eat them.
    /// - `--ordered` returns the selection in menu order rather than click order, and
    ///   `--height=24` bounds the menu so it cannot outgrow a popup pane.
    ///
    /// Rows go in over a pipe rather than as arguments so a very long registry cannot
    /// hit the `execve` argument limit. That is why this does not reuse `output()`.
    fn choose_projects(&mut self, rows: &[String]) -> Result<Option<String>> {
        let mut child = Command::new("gum")
            .args(review_menu_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to run gum")?;
        child
            .stdin
            .take()
            .context("failed to open gum input")?
            .write_all(format!("{}\n", rows.join("\n")).as_bytes())
            .context("failed to write gum input")?;
        let output = child.wait_with_output().context("failed to wait for gum")?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(
            String::from_utf8(output.stdout)
                .context("gum returned invalid UTF-8")?
                .trim_end_matches('\n')
                .to_owned(),
        ))
    }

    fn input(&mut self, header: &str, value: &str) -> Result<Option<String>> {
        Self::output(
            Command::new("gum")
                .arg("input")
                .arg(format!("--header={header}"))
                .arg(format!("--value={value}")),
        )
    }

    fn visibility(&mut self, current: &str) -> Result<Option<String>> {
        Self::output(
            Command::new("gum")
                .args(["choose", "visible", "hidden", "--header=Picker visibility"])
                .arg(format!("--selected={current}")),
        )
    }

    /// Wipe the screen before `edit` draws anything.
    ///
    /// `edit` is reached from the project picker's `ctrl-i` binding, and fzf owns the
    /// alternate screen for the whole life of the picker. A bound command inherits that
    /// screen with fzf's own drawing still on it, so the gum prompts would paint over
    /// the picker rows and fzf would redraw against a surface it no longer describes.
    /// Clearing first gives the editor the full pane and leaves fzf a clean screen to
    /// repaint when the command exits.
    ///
    /// `scripts/ssh-target-editor.zsh:33-35` already does this; the zsh project editor
    /// never did, and drew over the picker as a result. The parity contract's PIK-6
    /// makes it the rule for every binding-invoked editor, so this port adds it.
    fn clear(&mut self) -> Result<()> {
        Command::new("clear")
            .output()
            .context("failed to clear screen")?;
        Ok(())
    }
}

pub fn scan(registry_path: &Path) -> Result<()> {
    let config = crate::config::Config::load()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required")?;
    scan_with(
        registry_path,
        &home,
        Path::new(&config.projects_root),
        false,
        &SystemClock,
        &mut GumPrompt,
        &mut std::io::stdout(),
    )
}

pub fn rescan(registry_path: &Path) -> Result<()> {
    let config = crate::config::Config::load()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required")?;
    scan_with(
        registry_path,
        &home,
        Path::new(&config.projects_root),
        true,
        &SystemClock,
        &mut GumPrompt,
        &mut std::io::stdout(),
    )
}

/// Review discovered projects and write the ones that survive the menu.
///
/// `scan` and `rescan` differ only in where they start: `scan` refuses to run once a
/// registry exists, `rescan` requires one and marks the difference between it and
/// discovery.
///
/// **The write is all-or-nothing, and cancelling must never reach it.** The registry is
/// replaced by exactly the selected paths, so writing an empty selection would erase
/// the user's whole project list — every alias, every hidden flag, every
/// `last_used_at`. A cancelled menu and an empty selection therefore both bail before
/// `write_registry`, which is why the selection is checked twice: once for cancellation
/// and once for emptiness.
fn scan_with(
    registry_path: &Path,
    home: &Path,
    projects_root: &Path,
    rescan: bool,
    clock: &impl Clock,
    prompts: &mut impl PromptRunner,
    output: &mut impl Write,
) -> Result<()> {
    if rescan && !registry_path.is_file() {
        anyhow::bail!(
            "project-registry: registry does not exist: {}",
            registry_path.display()
        );
    }
    if !rescan && registry_path.exists() {
        anyhow::bail!(
            "project-registry: registry already exists: {}",
            registry_path.display()
        );
    }
    let existing = if rescan {
        read_registry(registry_path).map_err(|_| {
            anyhow::anyhow!(
                "project-registry: invalid registry: {}",
                registry_path.display()
            )
        })?
    } else {
        ProjectRegistry {
            version: 1,
            generated_at: String::new(),
            projects: BTreeMap::new(),
        }
    };
    let discovered = discovered_projects(home, projects_root);
    // Discovery, not the menu, is what has to be non-empty. `scripts/project-registry.zsh:268`
    // bails here whenever the three sources return nothing, and it does so on `rescan`
    // too — even with a populated registry. An empty discovery means the sources are
    // unreadable or the projects root is wrong, and reviewing an all-`[missing]` menu in
    // that state invites the user to confirm the deletion of everything.
    if discovered.is_empty() {
        anyhow::bail!("project-registry: no projects found");
    }
    let candidates = candidate_rows(&existing, &discovered, rescan);
    let rows = candidates
        .iter()
        .map(|item| item.row.clone())
        .collect::<Vec<_>>();
    let selection = prompts
        .choose_projects(&rows)?
        .ok_or_else(|| anyhow::anyhow!("project-registry: cancelled; registry not written"))?;

    // Drop rows with no tab, matching the zsh `awk -F '\t' 'NF >= 2 { print $NF }'`.
    // Only the last field is the path: a display name may itself contain a tab, and
    // parsing from the end is the only reading that survives one.
    let selected = selection
        .lines()
        .filter(|line| line.contains('\t'))
        .map(parse_selection_path)
        .collect::<BTreeSet<_>>();
    if selected.is_empty() {
        anyhow::bail!("project-registry: nothing selected; registry not written");
    }
    let mut projects = BTreeMap::new();
    for candidate in candidates {
        if !selected.contains(&candidate.path) {
            continue;
        }
        // Clone the registered entry so `aliases`, `hidden`, `last_used_at` and the
        // edited display name survive a `rescan`. Only `sources` is recomputed, and
        // only when discovery actually saw the path — a `[missing]` row that the user
        // keeps holds on to the sources it last had.
        let mut entry = existing
            .projects
            .get(&candidate.path)
            .cloned()
            .unwrap_or_else(|| ProjectEntry {
                name: file_name(Path::new(&candidate.path)).to_owned(),
                sources: discovered.get(&candidate.path).cloned().unwrap_or_default(),
                aliases: None,
                hidden: None,
                last_used_at: None,
            });
        if let Some(sources) = discovered.get(&candidate.path) {
            entry.sources = sources.clone();
        }
        projects.insert(candidate.path, entry);
    }
    let registry = ProjectRegistry {
        version: 1,
        generated_at: registry_timestamp(clock)?,
        projects,
    };
    write_registry(registry_path, &registry)?;
    writeln!(
        output,
        "project-registry: wrote {}",
        registry_path.display()
    )?;
    Ok(())
}

pub fn edit(registry_path: &Path, project_path: &Path) -> Result<()> {
    edit_with(
        registry_path,
        project_path,
        &mut GumPrompt,
        &mut std::io::stdout(),
    )
}

/// Edit one registered project: display name, aliases, picker visibility.
///
/// The three prompts run in that fixed order and each is seeded with the current value,
/// so pressing enter through all three is a no-op. Cancelling any one of them leaves
/// the registry untouched — the write happens once, after the last prompt returns.
fn edit_with(
    registry_path: &Path,
    project_path: &Path,
    prompts: &mut impl PromptRunner,
    output: &mut impl Write,
) -> Result<()> {
    if !registry_path.is_file() {
        anyhow::bail!(
            "project-registry: registry does not exist: {}",
            registry_path.display()
        );
    }
    if !project_path.is_absolute() {
        anyhow::bail!("project-registry: absolute project path is required");
    }
    let mut registry = read_registry(registry_path).map_err(|_| {
        anyhow::anyhow!(
            "project-registry: invalid registry: {}",
            registry_path.display()
        )
    })?;
    let key = project_path.to_string_lossy().into_owned();
    let entry = registry
        .projects
        .get(&key)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("project-registry: project is not registered: {key}"))?;
    prompts.clear()?;
    let cancelled = || anyhow::anyhow!("project-registry: edit cancelled; registry not written");
    let name = prompts
        .input("Display name", &entry.name)?
        .ok_or_else(&cancelled)?;
    let aliases = prompts
        .input(
            "Aliases (comma-separated)",
            &entry.aliases.clone().unwrap_or_default().join(", "),
        )?
        .ok_or_else(&cancelled)?;
    let current_visibility = if entry.hidden == Some(true) {
        "hidden"
    } else {
        "visible"
    };
    let visibility = prompts
        .visibility(current_visibility)?
        .ok_or_else(cancelled)?;
    anyhow::ensure!(
        matches!(visibility.as_str(), "visible" | "hidden"),
        "project-registry: invalid visibility; registry not written"
    );

    let edited = registry.projects.get_mut(&key).expect("entry was checked");
    // An emptied name falls back to the basename rather than being stored blank: the
    // picker renders this field, and a blank row cannot be selected by name.
    edited.name = if name.trim().is_empty() {
        file_name(project_path).to_owned()
    } else {
        name.trim().to_owned()
    };
    edited.aliases = Some(normalize_aliases(&aliases));
    edited.hidden = Some(visibility == "hidden");
    write_registry(registry_path, &registry)?;
    writeln!(
        output,
        "project-registry: edited {}",
        project_path.display()
    )?;
    Ok(())
}

pub fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("registry has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("registry has no file name: {}", path.display()))?
        .to_string_lossy();

    let mut temporary = None;
    for attempt in 0..1000_u16 {
        let candidate = parent.join(format!(".{file_name}.tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", candidate.display()));
            }
        }
    }
    let (temporary_path, temporary_file) =
        temporary.context("failed to create a temporary registry file")?;

    let result = (|| {
        let mut writer = BufWriter::new(temporary_file);
        serde_json::to_writer_pretty(&mut writer, value).context("failed to serialize registry")?;
        // serde_json omits the final newline, but jq writes one. Add it explicitly
        // so Rust and zsh registry files remain byte-identical.
        writer
            .write_all(b"\n")
            .context("failed to finish registry")?;
        writer.flush().context("failed to flush registry")?;
        drop(writer);
        fs::rename(&temporary_path, path)
            .with_context(|| format!("failed to replace {}", path.display()))
    })();

    if result.is_err() {
        // Rust's standard library has no trash equivalent. Removing only our
        // private temp file matches the parity diff and the zsh cleanup contract.
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

pub fn update(
    registry_path: &Path,
    home: &Path,
    projects_root: &Path,
    clock: &impl Clock,
) -> Result<()> {
    update_with_writer(
        registry_path,
        home,
        projects_root,
        clock,
        &mut std::io::stdout(),
    )
}

fn update_with_writer(
    registry_path: &Path,
    home: &Path,
    projects_root: &Path,
    clock: &impl Clock,
    output: &mut impl Write,
) -> Result<()> {
    if !registry_path.is_file() {
        anyhow::bail!(
            "project-registry: registry does not exist: {}",
            registry_path.display()
        );
    }
    let existing = read_registry(registry_path).map_err(|_| {
        anyhow::anyhow!(
            "project-registry: invalid registry: {}",
            registry_path.display()
        )
    })?;
    let discovered = discovered_projects(home, projects_root);
    let merged = merge_registry(existing, &discovered, registry_timestamp(clock)?);
    write_registry(registry_path, &merged)?;
    writeln!(
        output,
        "project-registry: updated {}",
        registry_path.display()
    )
    .context("failed to write update confirmation")?;
    Ok(())
}

pub fn use_project(
    registry_path: &Path,
    path: Option<&Path>,
    projects_root: &Path,
    clock: &impl Clock,
) -> Result<()> {
    if !registry_path.is_file() {
        anyhow::bail!(
            "project-registry: registry does not exist: {}",
            registry_path.display()
        );
    }
    let path = path.context("project-registry: project path is required")?;
    let project = canonical_project(path, projects_root).ok_or_else(|| {
        anyhow::anyhow!("project-registry: invalid project path: {}", path.display())
    })?;
    let mut registry = read_registry(registry_path).map_err(|_| {
        anyhow::anyhow!(
            "project-registry: invalid registry: {}",
            registry_path.display()
        )
    })?;
    let project = project.to_string_lossy().into_owned();
    let name = Path::new(&project)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let entry = registry
        .projects
        .entry(project)
        .or_insert_with(|| ProjectEntry {
            name,
            sources: vec!["manual".to_owned()],
            aliases: None,
            hidden: None,
            last_used_at: None,
        });
    entry.last_used_at = Some(clock.unix_seconds());
    write_registry(registry_path, &registry)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde::ser::Error as _;

    use super::*;

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct FixedClock(u64);

    impl Clock for FixedClock {
        fn unix_seconds(&self) -> u64 {
            self.0
        }
    }

    struct Fixture {
        directory: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self::new_in(&std::env::temp_dir(), label)
        }

        /// A fixture under an explicit base, so a test can pin a real `/tmp` path
        /// instead of relying on whatever `TMPDIR` happens to be.
        fn new_in(base: &Path, label: &str) -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let directory = base.join(format!(
                "workbench-project-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&directory).unwrap();
            Self { directory }
        }

        fn mkdir(&self, path: &str) -> PathBuf {
            let path = self.directory.join(path);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn write(&self, path: &str, contents: &str) -> PathBuf {
            let path = self.directory.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn git_repo(&self, path: &str) -> PathBuf {
            let path = self.mkdir(path);
            let status = Command::new("git")
                .args(["init", "--quiet"])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    fn registry_with(path: &str, sources: &[&str]) -> ProjectRegistry {
        ProjectRegistry {
            version: 1,
            generated_at: "old".to_owned(),
            projects: BTreeMap::from([(
                path.to_owned(),
                ProjectEntry {
                    name: "project".to_owned(),
                    sources: sources.iter().map(|source| (*source).to_owned()).collect(),
                    aliases: None,
                    hidden: None,
                    last_used_at: None,
                },
            )]),
        }
    }

    #[derive(Default)]
    struct FakePrompt {
        choices: VecDeque<Option<String>>,
        inputs: VecDeque<Option<String>>,
        visibilities: VecDeque<Option<String>>,
        cleared: bool,
    }

    impl PromptRunner for FakePrompt {
        fn choose_projects(&mut self, _rows: &[String]) -> Result<Option<String>> {
            Ok(self.choices.pop_front().flatten())
        }

        fn input(&mut self, _header: &str, _value: &str) -> Result<Option<String>> {
            Ok(self.inputs.pop_front().flatten())
        }

        fn visibility(&mut self, _current: &str) -> Result<Option<String>> {
            Ok(self.visibilities.pop_front().flatten())
        }

        fn clear(&mut self) -> Result<()> {
            self.cleared = true;
            Ok(())
        }
    }

    #[test]
    fn candidate_helpers_preserve_exact_rows_and_last_tab_field() {
        assert_eq!(
            format_candidate_row("project", "/projects/project", ""),
            "project\t/projects/project"
        );
        assert_eq!(
            format_candidate_row("project", "/projects/project", "[new]"),
            "[new] project\t/projects/project"
        );
        assert_eq!(
            parse_selection_path("[new] my\tproject\t/projects/project"),
            "/projects/project"
        );
    }

    #[test]
    fn candidate_rows_cover_scan_and_every_rescan_marker() {
        let entry = |name: &str| ProjectEntry {
            name: name.to_owned(),
            sources: vec![],
            aliases: None,
            hidden: None,
            last_used_at: None,
        };
        let existing = ProjectRegistry {
            version: 1,
            generated_at: String::new(),
            projects: BTreeMap::from([
                ("/projects/kept".to_owned(), entry("Kept Name")),
                ("/projects/missing".to_owned(), entry("Missing Name")),
            ]),
        };
        let discovered = BTreeMap::from([
            ("/projects/kept".to_owned(), vec!["claude".to_owned()]),
            ("/projects/new".to_owned(), vec!["codex".to_owned()]),
        ]);
        let empty = ProjectRegistry {
            version: 1,
            generated_at: String::new(),
            projects: BTreeMap::new(),
        };

        let scan = candidate_rows(&empty, &discovered, false)
            .into_iter()
            .map(|candidate| candidate.row)
            .collect::<Vec<_>>();
        assert_eq!(scan, ["kept\t/projects/kept", "new\t/projects/new"]);

        // The display column is the basename even for registered entries carrying an
        // edited name, and rows come out in path order — both as jq produces them.
        let rescan = candidate_rows(&existing, &discovered, true)
            .into_iter()
            .map(|candidate| candidate.row)
            .collect::<Vec<_>>();
        assert_eq!(
            rescan,
            [
                "kept\t/projects/kept",
                "[missing] missing\t/projects/missing",
                "[new] new\t/projects/new",
            ]
        );
    }

    #[test]
    fn aliases_are_trimmed_deemptied_and_deduplicated_in_order() {
        assert_eq!(
            normalize_aliases("  a  ,  b  ,  a  , , c, b "),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn cancelled_and_empty_scan_leave_registry_byte_for_byte_unchanged() {
        let fixture = Fixture::new("selection-integrity");
        let project = fixture.git_repo("projects/project");
        let registry_path = fixture.directory.join("registry.json");
        write_registry(
            &registry_path,
            &registry_with(project.to_str().unwrap(), &["manual"]),
        )
        .unwrap();
        let before = fs::read(&registry_path).unwrap();

        for choice in [None, Some(String::new())] {
            let mut prompt = FakePrompt {
                choices: VecDeque::from([choice]),
                ..FakePrompt::default()
            };
            let error = scan_with(
                &registry_path,
                &fixture.directory,
                &fixture.directory.join("projects"),
                true,
                &FixedClock(1_722_340_800),
                &mut prompt,
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("registry not written"));
            assert_eq!(fs::read(&registry_path).unwrap(), before);
        }
    }

    /// The guards that decide whether the review may run at all. `scan` is the
    /// destructive direction — it would replace a registry wholesale — so it refuses an
    /// existing file; `rescan` and `edit` refuse a missing one.
    #[test]
    fn review_guards_refuse_the_wrong_registry_state() {
        let fixture = Fixture::new("guards");
        let registry_path = fixture.directory.join("registry.json");
        let missing = fixture.directory.join("absent.json");
        let project = fixture.mkdir("projects/project");
        write_registry(
            &registry_path,
            &registry_with(project.to_str().unwrap(), &["manual"]),
        )
        .unwrap();

        let run = |path: &Path, rescan: bool| {
            scan_with(
                path,
                &fixture.directory,
                &fixture.directory.join("projects"),
                rescan,
                &FixedClock(1_722_340_800),
                &mut FakePrompt::default(),
                &mut Vec::new(),
            )
            .unwrap_err()
            .to_string()
        };

        assert_eq!(
            run(&registry_path, false),
            format!(
                "project-registry: registry already exists: {}",
                registry_path.display()
            )
        );
        assert_eq!(
            run(&missing, true),
            format!(
                "project-registry: registry does not exist: {}",
                missing.display()
            )
        );
        assert_eq!(
            edit_with(
                &missing,
                &project,
                &mut FakePrompt::default(),
                &mut Vec::new()
            )
            .unwrap_err()
            .to_string(),
            format!(
                "project-registry: registry does not exist: {}",
                missing.display()
            )
        );
    }

    /// Empty discovery aborts even when the registry is full, so a broken projects root
    /// can never present an all-`[missing]` menu.
    #[test]
    fn empty_discovery_aborts_rescan_before_prompting() {
        let fixture = Fixture::new("no-projects");
        let registry_path = fixture.directory.join("registry.json");
        fixture.mkdir("projects");
        write_registry(
            &registry_path,
            &registry_with("/projects/gone", &["manual"]),
        )
        .unwrap();

        let mut prompt = FakePrompt::default();
        let error = scan_with(
            &registry_path,
            &fixture.directory,
            &fixture.directory.join("projects"),
            true,
            &FixedClock(1_722_340_800),
            &mut prompt,
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();

        assert_eq!(error, "project-registry: no projects found");
    }

    /// The automated form of the task's manual check: run the real
    /// `scripts/project-registry.zsh rescan` over a fixture with a `gum` that captures
    /// its stdin and cancels, then compare that menu to `candidate_rows`. It pins the
    /// markers, the basename display column and the row order against the source of
    /// truth rather than against a transcription of it.
    #[test]
    fn zsh_and_rust_build_identical_rescan_menus() {
        let fixture = Fixture::new("menu-parity");
        let projects_root = fixture.mkdir("projects");
        let alpha = fs::canonicalize(fixture.git_repo("projects/alpha")).unwrap();
        let beta = fs::canonicalize(fixture.git_repo("projects/beta")).unwrap();
        let vanished = fixture.directory.join("vanished");

        let seed = ProjectRegistry {
            version: 1,
            generated_at: "2000-01-01T00:00:00Z".to_owned(),
            projects: BTreeMap::from([
                (
                    alpha.to_string_lossy().into_owned(),
                    ProjectEntry {
                        // A renamed entry: the menu must still show the basename.
                        name: "Alpha Edited".to_owned(),
                        sources: vec!["manual".to_owned()],
                        aliases: None,
                        hidden: None,
                        last_used_at: None,
                    },
                ),
                (
                    vanished.to_string_lossy().into_owned(),
                    ProjectEntry {
                        name: "Vanished".to_owned(),
                        sources: vec!["claude".to_owned()],
                        aliases: None,
                        hidden: None,
                        last_used_at: None,
                    },
                ),
            ]),
        };
        let registry = fixture.directory.join("registry.json");
        write_registry(&registry, &seed).unwrap();
        let before = fs::read(&registry).unwrap();

        // A `gum` that records the menu it was handed and exits non-zero, which is
        // exactly what the real one does when the user presses escape.
        let capture = fixture.directory.join("menu.txt");
        let arguments = fixture.directory.join("args.txt");
        let mock_bin = fixture.mkdir("bin");
        let gum = mock_bin.join("gum");
        fs::write(
            &gum,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > {}\ncat > {}\nexit 1\n",
                arguments.display(),
                capture.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&gum, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/project-registry.zsh");
        let path = format!(
            "{}:{}",
            mock_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let zsh = Command::new("zsh")
            .arg(script)
            .arg("rescan")
            .env("PATH", path)
            .env("HOME", &fixture.directory)
            .env("Q_WORKBENCH_LOCAL_CONFIG", "/dev/null")
            .env("Q_PROJECT_REGISTRY_FILE", &registry)
            .env("Q_PROJECTS_ROOT", &projects_root)
            .output()
            .unwrap();

        assert_eq!(
            String::from_utf8(zsh.stderr).unwrap(),
            "project-registry: cancelled; registry not written\n"
        );
        assert_eq!(fs::read(&registry).unwrap(), before);

        let zsh_rows = fs::read_to_string(&capture).unwrap();
        let zsh_rows = zsh_rows.lines().map(str::to_owned).collect::<Vec<_>>();
        let rust_rows = candidate_rows(
            &read_registry(&registry).unwrap(),
            &discovered_projects(&fixture.directory, &projects_root),
            true,
        )
        .into_iter()
        .map(|candidate| candidate.row)
        .collect::<Vec<_>>();

        assert_eq!(
            rust_rows,
            [
                format!("alpha\t{}", alpha.display()),
                format!("[new] beta\t{}", beta.display()),
                format!("[missing] vanished\t{}", vanished.display()),
            ]
        );
        assert_eq!(zsh_rows, rust_rows);

        // Flag parity. `--header value` and `--header=value` are the same flag to gum,
        // so both sides are folded to the `=` form before comparing; only the flag set
        // is contractual, not its order.
        let fold = |arguments: Vec<String>| {
            let mut folded = Vec::new();
            let mut arguments = arguments.into_iter().peekable();
            while let Some(argument) = arguments.next() {
                if argument == "--header" {
                    folded.push(format!("--header={}", arguments.next().unwrap_or_default()));
                } else {
                    folded.push(argument);
                }
            }
            folded.sort();
            folded
        };
        let zsh_arguments = fs::read_to_string(&arguments).unwrap();
        assert_eq!(
            fold(zsh_arguments.lines().map(str::to_owned).collect()),
            fold(review_menu_args())
        );
    }

    #[test]
    fn rescan_writes_only_selected_paths_and_preserves_edited_fields() {
        let fixture = Fixture::new("rescan-write");
        // Discovery reports canonical paths, and the registry is keyed by path, so the
        // fixture has to agree with it — `/var/folders` is a symlink on macOS.
        let alpha = fs::canonicalize(fixture.git_repo("projects/alpha")).unwrap();
        let beta = fs::canonicalize(fixture.git_repo("projects/beta")).unwrap();
        let registry_path = fixture.directory.join("registry.json");
        write_registry(
            &registry_path,
            &ProjectRegistry {
                version: 1,
                generated_at: "old".to_owned(),
                projects: BTreeMap::from([(
                    alpha.to_str().unwrap().to_owned(),
                    ProjectEntry {
                        name: "Alpha Edited".to_owned(),
                        sources: vec!["manual".to_owned()],
                        aliases: Some(vec!["a".to_owned()]),
                        hidden: Some(true),
                        last_used_at: Some(42),
                    },
                )]),
            },
        )
        .unwrap();

        // Keep alpha, drop the `[new]` beta row.
        let mut prompt = FakePrompt {
            choices: VecDeque::from([Some(format!("alpha\t{}", alpha.display()))]),
            ..FakePrompt::default()
        };
        let mut output = Vec::new();
        scan_with(
            &registry_path,
            &fixture.directory,
            &fixture.directory.join("projects"),
            true,
            &FixedClock(1_722_340_800),
            &mut prompt,
            &mut output,
        )
        .unwrap();

        let registry = read_registry(&registry_path).unwrap();
        assert_eq!(
            registry.projects.keys().collect::<Vec<_>>(),
            [alpha.to_str().unwrap()]
        );
        assert!(!registry.projects.contains_key(beta.to_str().unwrap()));
        let entry = &registry.projects[alpha.to_str().unwrap()];
        assert_eq!(entry.name, "Alpha Edited");
        assert_eq!(entry.aliases.as_ref().unwrap(), &["a"]);
        assert_eq!(entry.hidden, Some(true));
        assert_eq!(entry.last_used_at, Some(42));
        // `sources` is the one field the review recomputes; `manual` is not carried.
        assert_eq!(entry.sources, ["filesystem"]);
        assert_eq!(
            registry.generated_at,
            registry_timestamp(&FixedClock(1_722_340_800)).unwrap()
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("project-registry: wrote {}\n", registry_path.display())
        );
    }

    #[test]
    fn edit_refuses_a_path_that_is_not_registered() {
        let fixture = Fixture::new("edit-unregistered");
        let registry_path = fixture.directory.join("registry.json");
        let project = fixture.mkdir("projects/project");
        let other = fixture.mkdir("projects/other");
        write_registry(
            &registry_path,
            &registry_with(project.to_str().unwrap(), &["manual"]),
        )
        .unwrap();

        let error = edit_with(
            &registry_path,
            &other,
            &mut FakePrompt::default(),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            format!(
                "project-registry: project is not registered: {}",
                other.display()
            )
        );
    }

    #[test]
    fn edit_cancellation_preserves_registry_and_empty_name_uses_basename() {
        let fixture = Fixture::new("edit");
        let project = fixture.mkdir("projects/project");
        let registry_path = fixture.directory.join("registry.json");
        write_registry(
            &registry_path,
            &registry_with(project.to_str().unwrap(), &["manual"]),
        )
        .unwrap();
        let before = fs::read(&registry_path).unwrap();
        let mut cancelled = FakePrompt {
            inputs: VecDeque::from([None]),
            ..FakePrompt::default()
        };
        assert!(edit_with(&registry_path, &project, &mut cancelled, &mut Vec::new()).is_err());
        assert!(cancelled.cleared);
        assert_eq!(fs::read(&registry_path).unwrap(), before);

        let mut edited = FakePrompt {
            inputs: VecDeque::from([Some("   ".to_owned()), Some(" a, b, a, ".to_owned())]),
            visibilities: VecDeque::from([Some("hidden".to_owned())]),
            ..FakePrompt::default()
        };
        edit_with(&registry_path, &project, &mut edited, &mut Vec::new()).unwrap();
        let registry = read_registry(&registry_path).unwrap();
        let entry = &registry.projects[project.to_str().unwrap()];
        assert_eq!(entry.name, "project");
        assert_eq!(entry.aliases.as_ref().unwrap(), &["a", "b"]);
        assert_eq!(entry.hidden, Some(true));
    }

    #[test]
    fn merge_refreshes_existing_projects_without_adding_new_projects() {
        let registry = registry_with("/projects/kept", &["claude"]);
        let discovered = BTreeMap::from([
            (
                "/projects/kept".to_owned(),
                vec!["filesystem".to_owned(), "claude".to_owned()],
            ),
            ("/projects/new".to_owned(), vec!["filesystem".to_owned()]),
        ]);

        let merged = merge_registry(registry, &discovered, "pinned".to_owned());

        assert_eq!(merged.generated_at, "pinned");
        assert_eq!(
            merged.projects["/projects/kept"].sources,
            ["claude", "filesystem"]
        );
        assert!(!merged.projects.contains_key("/projects/new"));
    }

    #[test]
    fn merge_clears_sources_when_a_project_disappears() {
        let merged = merge_registry(
            registry_with("/projects/gone", &["claude", "filesystem"]),
            &BTreeMap::new(),
            "pinned".to_owned(),
        );

        assert!(merged.projects["/projects/gone"].sources.is_empty());
    }

    #[test]
    fn merge_preserves_manual_and_sorts_unique_sources() {
        let discovered = BTreeMap::from([(
            "/projects/manual".to_owned(),
            vec![
                "filesystem".to_owned(),
                "claude".to_owned(),
                "filesystem".to_owned(),
            ],
        )]);
        let merged = merge_registry(
            registry_with("/projects/manual", &["manual"]),
            &discovered,
            "pinned".to_owned(),
        );

        assert_eq!(
            merged.projects["/projects/manual"].sources,
            ["claude", "filesystem", "manual"]
        );
    }

    #[test]
    fn timestamp_uses_utc_without_subseconds_or_offset() {
        assert_eq!(
            registry_timestamp(&FixedClock(1_722_340_800)).unwrap(),
            "2024-07-30T12:00:00Z"
        );
    }

    #[test]
    fn atomic_write_is_pretty_printed_and_has_a_trailing_newline() {
        let fixture = Fixture::new("write");
        let path = fixture.directory.join("registry.json");

        write_registry(&path, &registry_with("/projects/a", &["manual"])).unwrap();

        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("\n  \"version\": 1,"));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn serialization_failure_keeps_the_registry_and_removes_the_temporary_file() {
        struct Broken;

        impl Serialize for Broken {
            fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(S::Error::custom("simulated failure"))
            }
        }

        let fixture = Fixture::new("failed-write");
        let path = fixture.write("registry.json", "original bytes\n");

        assert!(write_json_atomically(&path, &Broken).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original bytes\n");
        assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
    }

    #[test]
    fn operation_guards_use_the_contract_messages() {
        let fixture = Fixture::new("guards");
        let registry = fixture.directory.join("registry.json");
        let projects_root = fixture.mkdir("projects");
        let missing = update(
            &registry,
            &fixture.directory,
            &projects_root,
            &FixedClock(0),
        )
        .unwrap_err();
        assert_eq!(
            missing.to_string(),
            format!(
                "project-registry: registry does not exist: {}",
                registry.display()
            )
        );
        assert_eq!(
            use_project(&registry, None, &projects_root, &FixedClock(0))
                .unwrap_err()
                .to_string(),
            format!(
                "project-registry: registry does not exist: {}",
                registry.display()
            )
        );

        fixture.write("registry.json", "invalid");
        let invalid = update(
            &registry,
            &fixture.directory,
            &projects_root,
            &FixedClock(0),
        )
        .unwrap_err();
        assert_eq!(
            invalid.to_string(),
            format!("project-registry: invalid registry: {}", registry.display())
        );

        fs::write(
            &registry,
            serde_json::to_string(&ProjectRegistry {
                version: 1,
                generated_at: String::new(),
                projects: BTreeMap::new(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            use_project(&registry, None, &projects_root, &FixedClock(0))
                .unwrap_err()
                .to_string(),
            "project-registry: project path is required"
        );
        let invalid_path = fixture.directory.join("missing");
        assert_eq!(
            use_project(
                &registry,
                Some(&invalid_path),
                &projects_root,
                &FixedClock(0)
            )
            .unwrap_err()
            .to_string(),
            format!(
                "project-registry: invalid project path: {}",
                invalid_path.display()
            )
        );
    }

    #[test]
    fn update_prints_the_success_line_to_stdout() {
        let fixture = Fixture::new("update-output");
        let registry = fixture.directory.join("registry.json");
        let projects_root = fixture.mkdir("projects");
        write_registry(
            &registry,
            &ProjectRegistry {
                version: 1,
                generated_at: "old".to_owned(),
                projects: BTreeMap::new(),
            },
        )
        .unwrap();
        let mut output = Vec::new();

        update_with_writer(
            &registry,
            &fixture.directory,
            &projects_root,
            &FixedClock(0),
            &mut output,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("project-registry: updated {}\n", registry.display())
        );
    }

    #[test]
    fn zsh_and_rust_updates_write_byte_identical_registries() {
        let fixture = Fixture::new("update-parity");
        let projects_root = fixture.mkdir("projects");
        let alpha = fixture.git_repo("projects/alpha");
        let beta = fixture.git_repo("projects/beta");
        let vanished = fixture.directory.join("vanished");
        fixture.write(
            ".claude/projects/encoded/sessions-index.json",
            &format!(
                r#"{{"entries":[{{"projectPath":{}}}]}}"#,
                serde_json::to_string(&alpha).unwrap()
            ),
        );
        fixture.write(
            ".codex/sessions/2026/rollout-parity.jsonl",
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":{}}}}}\n",
                serde_json::to_string(&beta).unwrap()
            ),
        );

        let canonical_alpha = fs::canonicalize(&alpha)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let canonical_beta = fs::canonicalize(&beta)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let seed = ProjectRegistry {
            version: 1,
            generated_at: "2000-01-01T00:00:00Z".to_owned(),
            projects: BTreeMap::from([
                (
                    canonical_alpha,
                    ProjectEntry {
                        name: "Alpha".to_owned(),
                        sources: vec!["manual".to_owned()],
                        aliases: Some(vec!["a".to_owned(), "first".to_owned()]),
                        hidden: Some(true),
                        last_used_at: Some(123),
                    },
                ),
                (
                    canonical_beta,
                    ProjectEntry {
                        name: "Beta".to_owned(),
                        sources: vec!["codex".to_owned()],
                        aliases: None,
                        hidden: None,
                        last_used_at: None,
                    },
                ),
                (
                    vanished.to_string_lossy().into_owned(),
                    ProjectEntry {
                        name: "Vanished".to_owned(),
                        sources: vec!["claude".to_owned()],
                        aliases: None,
                        hidden: None,
                        last_used_at: None,
                    },
                ),
            ]),
        };
        let registry = fixture.directory.join("registry.json");
        write_registry(&registry, &seed).unwrap();

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/project-registry.zsh");
        let zsh = Command::new("zsh")
            .arg(script)
            .arg("update")
            .env("HOME", &fixture.directory)
            .env("Q_WORKBENCH_LOCAL_CONFIG", "/dev/null")
            .env("Q_PROJECT_REGISTRY_FILE", &registry)
            .env("Q_PROJECTS_ROOT", &projects_root)
            .output()
            .unwrap();
        assert!(
            zsh.status.success(),
            "zsh update failed: {}",
            String::from_utf8_lossy(&zsh.stderr)
        );
        assert_eq!(
            String::from_utf8(zsh.stdout).unwrap(),
            format!("project-registry: updated {}\n", registry.display())
        );
        let zsh_registry = fs::read(&registry).unwrap();

        write_registry(&registry, &seed).unwrap();
        let mut rust_stdout = Vec::new();
        update_with_writer(
            &registry,
            &fixture.directory,
            &projects_root,
            &FixedClock(1_722_340_800),
            &mut rust_stdout,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(rust_stdout).unwrap(),
            format!("project-registry: updated {}\n", registry.display())
        );
        let rust_registry = fs::read(&registry).unwrap();

        assert_eq!(
            normalize_generated_at(&zsh_registry),
            normalize_generated_at(&rust_registry)
        );
    }

    fn normalize_generated_at(contents: &[u8]) -> Vec<u8> {
        let contents = String::from_utf8(contents.to_vec()).unwrap();
        let prefix = "  \"generated_at\": \"";
        let value_start = contents.find(prefix).unwrap() + prefix.len();
        let value_end = value_start + contents[value_start..].find('"').unwrap();
        let mut normalized = contents;
        normalized.replace_range(value_start..value_end, "PINNED");
        normalized.into_bytes()
    }

    #[test]
    fn use_stamps_an_existing_project_and_creates_a_manual_project() {
        let fixture = Fixture::new("use");
        let projects_root = fixture.mkdir("projects");
        let project = fixture.mkdir("projects/new");
        let registry_path = fixture.directory.join("registry.json");
        write_registry(
            &registry_path,
            &ProjectRegistry {
                version: 1,
                generated_at: "pinned".to_owned(),
                projects: BTreeMap::new(),
            },
        )
        .unwrap();

        use_project(
            &registry_path,
            Some(&project),
            &projects_root,
            &FixedClock(123),
        )
        .unwrap();

        let registry = read_registry(&registry_path).unwrap();
        let canonical = fs::canonicalize(project)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(registry.projects[&canonical].sources, ["manual"]);
        assert_eq!(registry.projects[&canonical].last_used_at, Some(123));
    }

    #[test]
    fn test_canonical_project_resolves_git_toplevel() {
        let fixture = Fixture::new("canonical");
        let repo = fixture.git_repo("projects/repo");
        let subdirectory = fixture.mkdir("projects/repo/src/nested");
        let projects_root = fixture.directory.join("projects");

        assert_eq!(
            canonical_project(&subdirectory, &projects_root),
            Some(fs::canonicalize(&repo).unwrap())
        );
    }

    #[test]
    fn test_canonical_project_resolves_symlinks() {
        let fixture = Fixture::new("symlink");
        let repo = fixture.git_repo("projects/repo");
        let projects_root = fixture.directory.join("projects");
        let link = fixture.directory.join("projects/repo-link");
        std::os::unix::fs::symlink(&repo, &link).unwrap();

        assert_eq!(
            canonical_project(&link, &projects_root),
            Some(fs::canonicalize(&repo).unwrap())
        );
    }

    #[test]
    fn test_canonical_project_keeps_a_directory_that_is_not_a_repo() {
        let fixture = Fixture::new("plain");
        let plain = fixture.mkdir("projects/plain");
        let projects_root = fixture.directory.join("projects");

        assert_eq!(
            canonical_project(&plain, &projects_root),
            Some(fs::canonicalize(&plain).unwrap())
        );
    }

    #[test]
    fn test_canonical_project_rejects_root_and_missing_paths() {
        let fixture = Fixture::new("reject");
        let projects_root = fixture.mkdir("projects");

        assert_eq!(canonical_project(Path::new("/"), &projects_root), None);
        assert_eq!(
            canonical_project(&fixture.directory.join("gone"), &projects_root),
            None
        );
    }

    #[test]
    fn test_canonical_project_drops_tmp_paths() {
        // Pinned to a literal `/tmp` rather than `TMPDIR`, so the assertion does not
        // depend on where the platform puts temporary files. On macOS this also
        // covers `/private/tmp`, which is what `/tmp` canonicalises to.
        let fixture = Fixture::new_in(Path::new("/tmp"), "tmp-drop");
        let repo = fixture.git_repo("repo");
        let projects_root = Path::new(env!("CARGO_MANIFEST_DIR"));

        assert_eq!(canonical_project(&repo, projects_root), None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_canonical_project_drops_var_folders_paths() {
        let fixture = Fixture::new("var-folders-drop");
        // macOS gives every process a `TMPDIR` of `/var/folders/<x>/<y>/T/`; assert
        // the shape before relying on it.
        assert!(
            fixture.directory.starts_with("/var/folders"),
            "expected TMPDIR under /var/folders, got {:?}",
            fixture.directory
        );
        let repo = fixture.git_repo("repo");
        let projects_root = Path::new(env!("CARGO_MANIFEST_DIR"));

        assert_eq!(canonical_project(&repo, projects_root), None);
    }

    #[test]
    fn test_canonical_project_keeps_a_temp_path_inside_the_projects_root() {
        // The fixture root is itself temp-like (`TMPDIR` on macOS, `/tmp` elsewhere),
        // so this is exactly the case the projects-root exception exists for.
        let fixture = Fixture::new("inside-root");
        let projects_root = fixture.mkdir("projects");
        let repo = fixture.git_repo("projects/tmp-project");
        assert!(is_temporary_path(&fs::canonicalize(&repo).unwrap()));

        assert_eq!(
            canonical_project(&repo, &projects_root),
            Some(fs::canonicalize(&repo).unwrap())
        );
    }

    #[test]
    fn test_discover_claude() {
        let fixture = Fixture::new("claude");
        // Claude writes one index per encoded project directory; there is no
        // top-level index file.
        fixture.write(
            ".claude/projects/-Users-q-projects-alpha/sessions-index.json",
            r#"{"entries":[{"projectPath":"/projects/index-one"},{"projectPath":"/projects/index-two"}]}"#,
        );
        fixture.write(
            ".claude/projects/a/session.jsonl",
            "{\"message\":\"none\"}\n{\"cwd\":\"/projects/first\"}\n{\"cwd\":\"/projects/ignored\"}\n",
        );
        fixture.write(
            ".claude/projects/b/session.jsonl",
            "{\"payload\":{\"cwd\":\"/projects/second\"}}\n",
        );

        assert_eq!(
            discover_claude_projects(&fixture.directory),
            vec![
                (PathBuf::from("/projects/index-one"), Source::Claude),
                (PathBuf::from("/projects/index-two"), Source::Claude),
                (PathBuf::from("/projects/first"), Source::Claude),
                (PathBuf::from("/projects/second"), Source::Claude),
            ]
        );
    }

    #[test]
    fn test_discover_claude_skips_an_unreadable_transcript() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("claude-bad");
        // One unreadable file must not empty the whole source.
        let unreadable = fixture.write(".claude/projects/a/session.jsonl", "{\"cwd\":\"/gone\"}\n");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        fixture.write(".claude/projects/b/broken.jsonl", "not json at all\n");
        fixture.write(
            ".claude/projects/c/session.jsonl",
            "{\"cwd\":\"/projects/good\"}\n",
        );

        assert_eq!(
            discover_claude_projects(&fixture.directory),
            vec![(PathBuf::from("/projects/good"), Source::Claude)]
        );
    }

    #[test]
    fn test_discover_codex() {
        let fixture = Fixture::new("codex");
        fixture.write(
            ".codex/sessions/2026/rollout-a.jsonl",
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/projects/meta\"}}\n{\"cwd\":\"/projects/ignored\"}\n",
        );
        fixture.write(
            ".codex/sessions/2026/rollout-b.jsonl",
            "{\"cwd\":\"/projects/plain\"}\n",
        );
        fixture.write(
            ".codex/sessions/2026/not-a-rollout.jsonl",
            "{\"cwd\":\"/projects/wrong-file\"}\n",
        );

        assert_eq!(
            discover_codex_projects(&fixture.directory),
            vec![
                (PathBuf::from("/projects/meta"), Source::Codex),
                (PathBuf::from("/projects/plain"), Source::Codex),
            ]
        );
    }

    #[test]
    fn test_discover_filesystem_prunes_before_descending() {
        let fixture = Fixture::new("filesystem");
        fixture.mkdir("one/.git");
        fixture.mkdir("group/two/.git");
        fixture.mkdir("node_modules/dependency/.git");
        fixture.mkdir("group/vendor/dependency/.git");

        assert_eq!(
            discover_filesystem_projects(&fixture.directory),
            vec![
                (fixture.directory.join("group/two"), Source::Filesystem),
                (fixture.directory.join("one"), Source::Filesystem),
            ]
        );
    }

    #[test]
    fn test_discover_filesystem_finds_a_linked_worktree() {
        let fixture = Fixture::new("worktree");
        // A linked git worktree has a `.git` *file* pointing at the main checkout.
        fixture.write("worktree/.git", "gitdir: /elsewhere/.git/worktrees/wt\n");

        assert_eq!(
            discover_filesystem_projects(&fixture.directory),
            vec![(fixture.directory.join("worktree"), Source::Filesystem)]
        );
    }

    #[test]
    fn test_missing_source_dirs() {
        let fixture = Fixture::new("missing");

        assert!(discover_claude_projects(&fixture.directory).is_empty());
        assert!(discover_codex_projects(&fixture.directory).is_empty());
        assert!(discover_filesystem_projects(&fixture.directory.join("missing")).is_empty());
    }
}
