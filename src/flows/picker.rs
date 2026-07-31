//! The project and SSH pickers, including the project picker's fzf source.
//!
//! fzf binds `change:reload(<self> project source {q})`, so this module runs once per
//! keystroke. It makes no Herdr call and shells out only to `zoxide`, and only when a
//! query of two or more characters could produce a fallback row. `source_with_zoxide`
//! is pure apart from reading the registry, so the record bytes are tested directly.
//! Measured cost is in `docs/rust-rewrite/tasks/picker/01-project-picker-source.md`;
//! `scripts/bench-project-source.zsh` reproduces it.

use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde_json::json;

use crate::config::Config;
use crate::flows::agent::{self, InjectOptions};
use crate::herdr::HerdrClient;
use crate::notify::notify;
use crate::registry::project::{ProjectEntry, ProjectRegistry};
use crate::registry::ssh;
use crate::shell::shell_quote;

const PROJECT_ICON: &str = "󰉋";
const PROJECT_PICKER_TITLE: &str = "Project picker";
const PROJECT_MAIN_LABEL: &str = "󰧑  main";
const SSH_ICON: &str = "󰢩";
const SSH_PICKER_TITLE: &str = "SSH picker";

#[derive(Debug, PartialEq, Eq)]
struct ProjectSelection {
    query: String,
    key: String,
    path: String,
}

#[derive(Debug)]
pub struct ProjectPickerNotifiedError;

impl fmt::Display for ProjectPickerNotifiedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("project picker error was reported by notification")
    }
}

impl std::error::Error for ProjectPickerNotifiedError {}

pub fn project_pick(
    registry_path: &Path,
    projects_root: &Path,
    client: &dyn HerdrClient,
) -> Result<()> {
    if !registry_path.is_file() {
        let message = format!(
            "project picker: registry not found: {}",
            registry_path.display()
        );
        notify(client, PROJECT_PICKER_TITLE, &message);
        return Err(ProjectPickerNotifiedError.into());
    }

    adopt_invoking_cwd(client)?;
    let executable = std::env::current_exe().context("project pick: resolve current executable")?;
    let (reload_binding, edit_binding) = project_bindings(&executable);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required")?;
    let source = source(registry_path, &home, "")?;
    let mut child = Command::new("fzf")
        .args(project_fzf_args(&reload_binding, &edit_binding))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("project pick: start fzf")?;
    child
        .stdin
        .take()
        .context("project pick: open fzf input")?
        .write_all(&source)
        .context("project pick: write fzf input")?;
    let output = child
        .wait_with_output()
        .context("project pick: wait for fzf")?;
    if !output.status.success() {
        return Ok(());
    }

    let output = String::from_utf8(output.stdout).context("project pick: parse fzf output")?;
    let selection = parse_project_selection(&output);
    let path = match resolve_project_path(&selection, query_zoxide)? {
        Some(path) => path,
        None => {
            let missing = if selection.query.is_empty() {
                &selection.path
            } else {
                &selection.query
            };
            let message = format!("project picker: project not found: {missing}");
            notify(client, PROJECT_PICKER_TITLE, &message);
            return Err(ProjectPickerNotifiedError.into());
        }
    };
    focus_or_create_project(&path, &selection.key, registry_path, client)?;
    crate::registry::project::use_project(
        registry_path,
        Some(&path),
        projects_root,
        &crate::registry::project::SystemClock,
    )
}

fn parse_project_selection(output: &str) -> ProjectSelection {
    let mut lines = output.split('\n');
    ProjectSelection {
        query: lines
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r')
            .to_owned(),
        key: lines
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r')
            .to_owned(),
        path: lines
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r')
            .to_owned(),
    }
}

fn project_bindings(executable: &Path) -> (String, String) {
    let executable = shell_quote(&executable.to_string_lossy());
    (
        format!("change:reload({executable} project source {{q}})"),
        format!(
            "ctrl-i:execute({executable} project edit {{2}})+reload({executable} project source {{q}})"
        ),
    )
}

fn project_fzf_args<'a>(reload_binding: &'a str, edit_binding: &'a str) -> Vec<&'a str> {
    vec![
        "--read0",
        "--print-query",
        "--expect=alt-enter",
        "--prompt=Project> ",
        "--highlight-line",
        "--pointer=▌",
        "--info=inline-right",
        "--delimiter=\t",
        "--with-nth=1",
        "--accept-nth=2",
        "--bind",
        reload_binding,
        "--bind",
        edit_binding,
        "--border",
        "--border-label-pos=bottom",
        "--border-label= enter: agent · alt-enter: plain · ctrl-i: edit · typing searches zoxide ",
    ]
}

fn resolve_project_path(
    selection: &ProjectSelection,
    zoxide: impl FnOnce(&str) -> Result<Option<PathBuf>>,
) -> Result<Option<PathBuf>> {
    let candidate = if !selection.path.is_empty() {
        Some(PathBuf::from(&selection.path))
    } else if selection.query.is_empty() {
        None
    } else {
        let query_path = PathBuf::from(&selection.query);
        if query_path.is_dir() {
            Some(query_path)
        } else {
            zoxide(&selection.query)?
        }
    };
    let Some(candidate) = candidate.filter(|path| path.is_dir()) else {
        return Ok(None);
    };
    fs::canonicalize(&candidate)
        .map(Some)
        .with_context(|| format!("project pick: resolve {}", candidate.display()))
}

fn focus_or_create_project(
    path: &Path,
    key: &str,
    registry_path: &Path,
    client: &dyn HerdrClient,
) -> Result<()> {
    let snapshot = client
        .session_snapshot(json!({}))
        .context("project pick: session.snapshot")?;
    let path_text = path.to_string_lossy();
    let workspace_id = snapshot
        .fields
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("panes"))
        .and_then(|panes| panes.as_array())
        .and_then(|panes| {
            panes.iter().find_map(|pane| {
                let matches = pane.get("cwd").and_then(|value| value.as_str())
                    == Some(path_text.as_ref())
                    || pane.get("foreground_cwd").and_then(|value| value.as_str())
                        == Some(path_text.as_ref());
                matches
                    .then(|| pane.get("workspace_id")?.as_str().map(str::to_owned))
                    .flatten()
            })
        });

    // When a workspace already exists for this path, we only focus it without
    // creating an agent tab. This asymmetry is intentional: the enter key shows
    // the agent menu and injects a launcher, while picking an existing project
    // reuses it as-is. Only new workspaces get the agent tab.
    let workspace_id = if let Some(workspace_id) = workspace_id {
        workspace_id
    } else {
        let label = project_label(registry_path, path)?;
        let created = client
            .workspace_create(json!({
                "cwd": path_text,
                "env": {"Q_NO_BANNER": "1"},
                "focus": false,
                "label": label,
            }))
            .context("project pick: workspace.create")?;
        let workspace_id = created.workspace.workspace_id;
        if key != "alt-enter" {
            client
                .tab_rename(json!({
                    "tab_id": created.tab.tab_id,
                    "label": PROJECT_MAIN_LABEL,
                }))
                .context("project pick: tab.rename")?;
            agent::inject(
                client,
                &InjectOptions {
                    pane_id: created.root_pane.pane_id,
                    tab_id: None,
                    usage: Some(PROJECT_MAIN_LABEL.to_owned()),
                    worktree: false,
                },
            )
            .context("project pick: inject agent")?;
        }
        workspace_id
    };
    client
        .workspace_focus(json!({"workspace_id": workspace_id}))
        .context("project pick: workspace.focus")?;
    Ok(())
}

fn project_label(registry_path: &Path, path: &Path) -> Result<String> {
    let registry = crate::registry::project::read_registry(registry_path)?;
    Ok(registry
        .projects
        .get(path.to_string_lossy().as_ref())
        .map(|entry| entry.name.clone())
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default())
}

fn adopt_invoking_cwd(client: &dyn HerdrClient) -> Result<()> {
    let context_cwd = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
        .and_then(|value| value.get("focused_pane_cwd")?.as_str().map(PathBuf::from));
    let cwd = context_cwd.filter(|path| path.is_dir()).or_else(|| {
        std::env::var("HERDR_ACTIVE_PANE_ID")
            .ok()
            .and_then(|pane_id| client.pane_get(json!({"pane_id": pane_id})).ok())
            .and_then(|response| response.pane.foreground_cwd.or(response.pane.cwd))
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
    });
    if let Some(cwd) = cwd {
        std::env::set_current_dir(&cwd)
            .with_context(|| format!("project pick: adopt invoking cwd {}", cwd.display()))?;
    }
    Ok(())
}

pub fn ssh_pick(
    registry: &Path,
    config: &Path,
    history: &Path,
    client: &dyn HerdrClient,
) -> Result<()> {
    let entries = ssh::list(registry, config, history).context("ssh pick: list targets")?;
    let executable = std::env::current_exe().context("ssh pick: resolve current executable")?;
    let (edit_binding, remove_binding) = ssh_bindings(&executable);
    let mut child = Command::new("fzf")
        .args(ssh_fzf_args(&edit_binding, &remove_binding))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("ssh pick: start fzf")?;
    child
        .stdin
        .take()
        .context("ssh pick: open fzf input")?
        .write_all(&entries)
        .context("ssh pick: write fzf input")?;
    let output = child.wait_with_output().context("ssh pick: wait for fzf")?;
    if !output.status.success() {
        return Ok(());
    }

    let output = String::from_utf8(output.stdout).context("ssh pick: parse fzf output")?;
    let Some(target) = parse_ssh_selection(&output) else {
        return Ok(());
    };
    connect_ssh_target(&target, &executable, client)
}

fn parse_ssh_selection(output: &str) -> Option<String> {
    let mut lines = output.lines();
    let query = lines.next().unwrap_or_default().trim_end_matches('\r');
    let selection = lines.next().unwrap_or_default().trim_end_matches('\r');
    let target = if selection.is_empty() {
        query
    } else {
        selection
    };
    (!target.is_empty()).then(|| target.to_owned())
}

fn ssh_bindings(executable: &Path) -> (String, String) {
    let executable = shell_quote(&executable.to_string_lossy());
    (
        format!("ctrl-i:execute({executable} ssh edit {{2}})+reload({executable} ssh list)"),
        format!(
            "ctrl-x:execute-silent({executable} ssh remove {{2}})+reload({executable} ssh list)"
        ),
    )
}

fn ssh_fzf_args<'a>(edit_binding: &'a str, remove_binding: &'a str) -> Vec<&'a str> {
    vec![
        "--no-sort",
        "--print-query",
        "--prompt=SSH> ",
        "--read0",
        "--highlight-line",
        "--gap",
        "--gap-line",
        "--pointer=▌",
        "--border",
        "--border-label-pos=bottom",
        "--border-label= enter: connect · ctrl-i: edit · ctrl-x: remove ",
        "--delimiter=\t",
        "--with-nth=1",
        "--accept-nth=2",
        "--bind",
        edit_binding,
        "--bind",
        remove_binding,
    ]
}

fn session_command(executable: &Path, target: &str, tab_id: &str) -> String {
    // The pane's interactive shell interprets the text sent via pane.send_input, so every
    // embedded value must be shell-quoted to prevent field splitting and injection attacks.
    [
        executable.to_string_lossy().into_owned(),
        "ssh".to_owned(),
        "session".to_owned(),
        target.to_owned(),
        tab_id.to_owned(),
    ]
    .iter()
    .map(|part| shell_quote(part))
    .collect::<Vec<_>>()
    .join(" ")
}

fn connect_ssh_target(target: &str, executable: &Path, client: &dyn HerdrClient) -> Result<()> {
    let create_params =
        ssh_tab_create_params(target, std::env::var("HERDR_WORKSPACE_ID").ok().as_deref());
    let created = match client.tab_create(create_params) {
        Ok(created) => created,
        Err(error) => return notify_ssh_error(client, error.context("ssh pick: tab.create")),
    };
    let tab_id = created.tab.tab_id;
    let command = session_command(executable, target, &tab_id);

    if let Err(error) = client.pane_send_input(json!({
        "pane_id": created.root_pane.pane_id,
        "text": command,
        "keys": ["enter"],
    })) {
        close_failed_ssh_tab(client, &tab_id);
        return notify_ssh_error(client, error.context("ssh pick: pane.send_input"));
    }
    if let Err(error) = client.tab_focus(json!({"tab_id": tab_id})) {
        close_failed_ssh_tab(client, &tab_id);
        return notify_ssh_error(client, error.context("ssh pick: tab.focus"));
    }
    Ok(())
}

fn ssh_tab_create_params(target: &str, workspace_id: Option<&str>) -> serde_json::Value {
    let mut params = json!({
        "label": format!("{SSH_ICON}  {target}"),
        "env": {"Q_NO_BANNER": "1"},
        "focus": false,
    });
    if let Some(workspace_id) = workspace_id {
        params["workspace_id"] = json!(workspace_id);
    }
    params
}

fn close_failed_ssh_tab(client: &dyn HerdrClient, tab_id: &str) {
    let _ = client.tab_close(json!({"tab_id": tab_id}));
}

fn notify_ssh_error(client: &dyn HerdrClient, error: anyhow::Error) -> Result<()> {
    notify(client, SSH_PICKER_TITLE, &format!("{error:#}"));
    Err(anyhow!(error))
}

pub fn project_source(query: Option<&str>) -> Result<()> {
    let config = Config::load().context("failed to load config")?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required")?;
    let output = source(
        Path::new(&config.project_registry_file),
        &home,
        query.unwrap_or(""),
    )?;
    io::stdout()
        .write_all(&output)
        .context("failed to write project source")
}

fn source(registry_path: &Path, home: &Path, query: &str) -> Result<Vec<u8>> {
    source_with_zoxide(registry_path, home, query, query_zoxide)
}

fn source_with_zoxide(
    registry_path: &Path,
    home: &Path,
    query: &str,
    zoxide: impl FnOnce(&str) -> Result<Option<PathBuf>>,
) -> Result<Vec<u8>> {
    let contents = fs::read(registry_path)
        .with_context(|| format!("failed to read {}", registry_path.display()))?;
    let registry: ProjectRegistry = serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse {}", registry_path.display()))?;

    let mut projects = registry
        .projects
        .iter()
        .filter(|(_, entry)| entry.hidden != Some(true))
        .map(|(path, entry)| ProjectRow {
            path,
            entry,
            display_name: display_name(entry),
        })
        .collect::<Vec<_>>();
    projects.sort_unstable_by(compare_projects);

    let mut output = Vec::with_capacity(projects.len() * 128);
    for project in projects {
        push_record(
            &mut output,
            project.entry,
            &project.display_name,
            Path::new(project.path),
            home,
        );
    }

    if query.chars().count() < 2 {
        return Ok(output);
    }
    let Some(path) = zoxide(query)? else {
        return Ok(output);
    };
    if !path.is_dir() {
        return Ok(output);
    }
    let path = fs::canonicalize(&path)
        .with_context(|| format!("failed to resolve zoxide path {}", path.display()))?;
    if registry
        .projects
        .contains_key(&path.to_string_lossy().into_owned())
    {
        return Ok(output);
    }

    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(output);
    };
    let entry = ProjectEntry {
        name: name.to_owned(),
        sources: vec!["zoxide".to_owned()],
        aliases: None,
        hidden: None,
        last_used_at: None,
    };
    push_record(&mut output, &entry, name, &path, home);
    Ok(output)
}

struct ProjectRow<'a> {
    path: &'a str,
    entry: &'a ProjectEntry,
    display_name: String,
}

fn compare_projects(left: &ProjectRow<'_>, right: &ProjectRow<'_>) -> Ordering {
    let left_entry = left.entry;
    let right_entry = right.entry;
    match (left_entry.last_used_at, right_entry.last_used_at) {
        (Some(left_time), Some(right_time)) => right_time
            .cmp(&left_time)
            .then_with(|| left.display_name.cmp(&right.display_name)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.display_name.cmp(&right.display_name),
    }
}

fn display_name(entry: &ProjectEntry) -> String {
    let mut name = entry.name.clone();
    if let Some(aliases) = &entry.aliases {
        for alias in aliases {
            name.push_str(" | ");
            name.push_str(alias);
        }
    }
    name
}

fn push_record(
    output: &mut Vec<u8>,
    entry: &ProjectEntry,
    display_name: &str,
    path: &Path,
    home: &Path,
) {
    output.extend_from_slice(PROJECT_ICON.as_bytes());
    output.extend_from_slice(b"  ");
    output.extend_from_slice(display_name.as_bytes());
    output.extend_from_slice(b"\n   ");
    push_display_path(output, path, home);
    output.extend_from_slice(b"\n   ");
    for (index, source) in entry.sources.iter().enumerate() {
        if index != 0 {
            output.extend_from_slice(" · ".as_bytes());
        }
        output.extend_from_slice(source.as_bytes());
    }
    output.push(b'\t');
    output.extend_from_slice(path.as_os_str().as_encoded_bytes());
    output.push(0);
}

fn push_display_path(output: &mut Vec<u8>, path: &Path, home: &Path) {
    if path == home {
        output.push(b'~');
        return;
    }
    match path.strip_prefix(home) {
        Ok(relative) => {
            output.extend_from_slice(b"~/");
            output.extend_from_slice(relative.as_os_str().as_encoded_bytes());
        }
        Err(_) => output.extend_from_slice(path.as_os_str().as_encoded_bytes()),
    }
}

fn query_zoxide(query: &str) -> Result<Option<PathBuf>> {
    let output = match Command::new("zoxide").args(["query", "--", query]).output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to execute zoxide"),
    };
    if !output.status.success() {
        return Ok(None);
    }

    let path = String::from_utf8(output.stdout).context("zoxide returned a non-UTF-8 path")?;
    let path = path.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::herdr::FakeClient;

    #[test]
    fn parses_selection_then_falls_back_to_query() {
        assert_eq!(
            parse_ssh_selection("typed\nselected\n"),
            Some("selected".to_owned())
        );
        assert_eq!(
            parse_ssh_selection("unlisted-host\n"),
            Some("unlisted-host".to_owned())
        );
        assert_eq!(parse_ssh_selection("\n"), None);
        assert_eq!(parse_ssh_selection(""), None);
    }

    #[test]
    fn bindings_quote_an_executable_path_containing_spaces() {
        let executable = Path::new("/Applications/Herdr Workbench/workbench");
        let (edit, remove) = ssh_bindings(executable);

        assert_eq!(
            edit,
            "ctrl-i:execute('/Applications/Herdr Workbench/workbench' ssh edit {2})+reload('/Applications/Herdr Workbench/workbench' ssh list)"
        );
        assert_eq!(
            remove,
            "ctrl-x:execute-silent('/Applications/Herdr Workbench/workbench' ssh remove {2})+reload('/Applications/Herdr Workbench/workbench' ssh list)"
        );
    }

    #[test]
    fn ssh_fzf_arguments_match_the_parity_contract() {
        let edit = "ctrl-i:execute(<self> ssh edit {2})+reload(<self> ssh list)";
        let remove = "ctrl-x:execute-silent(<self> ssh remove {2})+reload(<self> ssh list)";

        assert_eq!(
            ssh_fzf_args(edit, remove),
            vec![
                "--no-sort",
                "--print-query",
                "--prompt=SSH> ",
                "--read0",
                "--highlight-line",
                "--gap",
                "--gap-line",
                "--pointer=▌",
                "--border",
                "--border-label-pos=bottom",
                "--border-label= enter: connect · ctrl-i: edit · ctrl-x: remove ",
                "--delimiter=\t",
                "--with-nth=1",
                "--accept-nth=2",
                "--bind",
                edit,
                "--bind",
                remove,
            ]
        );
    }

    #[test]
    fn parses_project_result_positionally_for_query_and_key_combinations() {
        let cases = [
            ("\n\n/project\n", "", ""),
            ("typed\n\n/project\n", "typed", ""),
            ("\nalt-enter\n/project\n", "", "alt-enter"),
            ("typed\nalt-enter\n/project\n", "typed", "alt-enter"),
        ];

        for (output, query, key) in cases {
            assert_eq!(
                parse_project_selection(output),
                ProjectSelection {
                    query: query.to_owned(),
                    key: key.to_owned(),
                    path: "/project".to_owned(),
                }
            );
        }
    }

    #[test]
    fn project_bindings_quote_an_executable_path_containing_spaces() {
        let executable = Path::new("/Applications/Herdr Workbench/workbench");
        let (reload, edit) = project_bindings(executable);

        assert_eq!(
            reload,
            "change:reload('/Applications/Herdr Workbench/workbench' project source {q})"
        );
        assert_eq!(
            edit,
            "ctrl-i:execute('/Applications/Herdr Workbench/workbench' project edit {2})+reload('/Applications/Herdr Workbench/workbench' project source {q})"
        );
    }

    #[test]
    fn project_fzf_arguments_match_the_parity_contract() {
        let reload = "change:reload(<self> project source {q})";
        let edit = "ctrl-i:execute(<self> project edit {2})+reload(<self> project source {q})";

        assert_eq!(
            project_fzf_args(reload, edit),
            vec![
                "--read0",
                "--print-query",
                "--expect=alt-enter",
                "--prompt=Project> ",
                "--highlight-line",
                "--pointer=▌",
                "--info=inline-right",
                "--delimiter=\t",
                "--with-nth=1",
                "--accept-nth=2",
                "--bind",
                reload,
                "--bind",
                edit,
                "--border",
                "--border-label-pos=bottom",
                "--border-label= enter: agent · alt-enter: plain · ctrl-i: edit · typing searches zoxide ",
            ]
        );
    }

    fn queue_snapshot(client: &FakeClient, panes: serde_json::Value) {
        client.queue_response(
            "session.snapshot",
            json!({"type": "session_snapshot", "snapshot": {"panes": panes}}),
        );
    }

    fn queue_created_workspace(client: &FakeClient) {
        client.queue_response(
            "workspace.create",
            json!({
                "type": "workspace_created",
                "workspace": {"workspace_id": "w-new"},
                "tab": {"tab_id": "t-new"},
                "root_pane": {"pane_id": "p-new"},
            }),
        );
    }

    #[test]
    fn existing_project_workspace_is_only_focused() {
        let directory = TestDirectory::new();
        let project = directory.0.join("project");
        fs::create_dir(&project).expect("create project");
        let registry = write_registry(
            &directory.0,
            [(
                project.to_string_lossy().into_owned(),
                entry("project", &["manual"], &[], false, None),
            )]
            .into_iter()
            .collect(),
        );
        let client = FakeClient::default();
        queue_snapshot(&client, json!([{"workspace_id": "w-open", "cwd": project}]));

        focus_or_create_project(&project, "", &registry, &client).expect("focus workspace");

        assert_eq!(
            client
                .calls
                .into_inner()
                .into_iter()
                .map(|(method, _)| method)
                .collect::<Vec<_>>(),
            ["session.snapshot", "workspace.focus"]
        );
    }

    #[test]
    fn new_project_enter_creates_injects_then_focuses() {
        let directory = TestDirectory::new();
        let project = directory.0.join("project");
        fs::create_dir(&project).expect("create project");
        let registry = write_registry(
            &directory.0,
            [(
                project.to_string_lossy().into_owned(),
                entry("Project label", &["manual"], &[], false, None),
            )]
            .into_iter()
            .collect(),
        );
        let client = FakeClient::default();
        queue_snapshot(&client, json!([]));
        queue_created_workspace(&client);

        focus_or_create_project(&project, "", &registry, &client).expect("create workspace");

        let calls = client.calls.into_inner();
        assert_eq!(
            calls
                .iter()
                .map(|(method, _)| method.as_str())
                .collect::<Vec<_>>(),
            [
                "session.snapshot",
                "workspace.create",
                "tab.rename",
                "pane.rename",
                "pane.send_input",
                "workspace.focus",
            ]
        );
        assert_eq!(
            calls[2].1,
            json!({"tab_id": "t-new", "label": PROJECT_MAIN_LABEL})
        );
        assert!(calls[4].1["text"]
            .as_str()
            .expect("launcher command")
            .contains("'--usage' '󰧑  main'"));
    }

    #[test]
    fn new_project_alt_enter_creates_plain_workspace_then_focuses() {
        let directory = TestDirectory::new();
        let project = directory.0.join("project");
        fs::create_dir(&project).expect("create project");
        let registry = write_registry(&directory.0, BTreeMap::new());
        let client = FakeClient::default();
        queue_snapshot(&client, json!([]));
        queue_created_workspace(&client);

        focus_or_create_project(&project, "alt-enter", &registry, &client)
            .expect("create plain workspace");

        assert_eq!(
            client
                .calls
                .into_inner()
                .into_iter()
                .map(|(method, _)| method)
                .collect::<Vec<_>>(),
            ["session.snapshot", "workspace.create", "workspace.focus"]
        );
    }

    fn queue_created_tab(client: &FakeClient) {
        client.queue_response(
            "tab.create",
            json!({
                "type": "tab.created",
                "root_pane": {"pane_id": "p-root"},
                "tab": {"tab_id": "t-ssh"}
            }),
        );
    }

    #[test]
    fn connecting_creates_sends_then_focuses() {
        let client = FakeClient::default();
        queue_created_tab(&client);

        connect_ssh_target(
            "server's host",
            Path::new("/Applications/Herdr Workbench/workbench"),
            &client,
        )
        .expect("connect target");

        let calls = client.calls.into_inner();
        assert_eq!(calls[0].0, "tab.create");
        assert_eq!(
            calls[0].1,
            ssh_tab_create_params(
                "server's host",
                std::env::var("HERDR_WORKSPACE_ID").ok().as_deref(),
            )
        );
        assert_eq!(calls[1].0, "pane.send_input");
        assert_eq!(
            calls[1].1,
            json!({
                "pane_id": "p-root",
                "text": "'/Applications/Herdr Workbench/workbench' 'ssh' 'session' 'server'\\''s host' 't-ssh'",
                "keys": ["enter"],
            })
        );
        assert_eq!(
            calls[2],
            ("tab.focus".to_owned(), json!({"tab_id": "t-ssh"}))
        );
    }

    #[test]
    fn tab_creation_includes_the_current_workspace_when_present() {
        assert_eq!(
            ssh_tab_create_params("host", Some("workspace-7")),
            json!({
                "label": "󰢩  host",
                "env": {"Q_NO_BANNER": "1"},
                "focus": false,
                "workspace_id": "workspace-7",
            })
        );
    }

    #[test]
    fn send_failure_closes_the_created_tab() {
        let client = FakeClient::default();
        queue_created_tab(&client);
        client.queue_error("pane.send_input", "unavailable", "socket closed");

        connect_ssh_target("host", Path::new("/bin/workbench"), &client)
            .expect_err("reject send failure");

        assert_eq!(
            client
                .calls
                .into_inner()
                .into_iter()
                .map(|(method, _)| method)
                .collect::<Vec<_>>(),
            [
                "tab.create",
                "pane.send_input",
                "tab.close",
                "notification.show"
            ]
        );
    }

    #[test]
    fn focus_failure_closes_the_created_tab() {
        let client = FakeClient::default();
        queue_created_tab(&client);
        client.queue_error("tab.focus", "unavailable", "socket closed");

        connect_ssh_target("host", Path::new("/bin/workbench"), &client)
            .expect_err("reject focus failure");

        assert_eq!(
            client
                .calls
                .into_inner()
                .into_iter()
                .map(|(method, _)| method)
                .collect::<Vec<_>>(),
            [
                "tab.create",
                "pane.send_input",
                "tab.focus",
                "tab.close",
                "notification.show"
            ]
        );
    }

    #[test]
    fn session_command_round_trips_every_shell_quoted_part() {
        let executable = Path::new("/Applications/Herdr Workbench/workbench");
        let command = session_command(executable, "user's host;$HOME", "tab id'1");
        let script = format!("set -- {command}; printf '%s\\0' \"$@\"");
        let output = Command::new("zsh")
            .args(["-c", &script])
            .output()
            .expect("zsh must be available");

        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            b"/Applications/Herdr Workbench/workbench\0ssh\0session\0user's host;$HOME\0tab id'1\0"
        );
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is before Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("workbench-picker-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }

    fn entry(
        name: &str,
        sources: &[&str],
        aliases: &[&str],
        hidden: bool,
        last_used_at: Option<u64>,
    ) -> ProjectEntry {
        ProjectEntry {
            name: name.to_owned(),
            sources: sources.iter().map(|value| (*value).to_owned()).collect(),
            aliases: (!aliases.is_empty())
                .then(|| aliases.iter().map(|value| (*value).to_owned()).collect()),
            hidden: hidden.then_some(true),
            last_used_at,
        }
    }

    fn write_registry(directory: &Path, projects: BTreeMap<String, ProjectEntry>) -> PathBuf {
        let path = directory.join("registry.json");
        let registry = ProjectRegistry {
            version: 1,
            generated_at: "2026-07-30T12:00:00Z".to_owned(),
            projects,
        };
        fs::write(
            &path,
            serde_json::to_vec(&registry).expect("serialize registry"),
        )
        .expect("write registry");
        path
    }

    #[test]
    fn emits_exact_three_line_nul_delimited_records_in_picker_order() {
        let directory = TestDirectory::new();
        let home = directory.0.join("home");
        let used = home.join("Used");
        let alpha = home.join("Alpha");
        let aliased = home.join("Aliased");
        let hidden = home.join("Hidden");
        let projects = [
            (
                alpha.display().to_string(),
                entry("Alpha", &["codex"], &[], false, None),
            ),
            (
                aliased.display().to_string(),
                entry("Beta", &["claude", "filesystem"], &["A", "C"], false, None),
            ),
            (
                hidden.display().to_string(),
                entry("Hidden", &["codex"], &[], true, Some(99)),
            ),
            (
                used.display().to_string(),
                entry("Used", &["codex"], &[], false, Some(10)),
            ),
        ]
        .into_iter()
        .collect();
        let registry = write_registry(&directory.0, projects);

        let output = source_with_zoxide(&registry, &home, "", |_| Ok(None)).expect("emit source");
        let expected = "\u{f024b}  Used\n   ~/Used\n   codex\t"
            .as_bytes()
            .iter()
            .copied()
            .chain(used.display().to_string().bytes())
            .chain([0])
            .chain("󰉋  Alpha\n   ~/Alpha\n   codex\t".bytes())
            .chain(alpha.display().to_string().bytes())
            .chain([0])
            .chain("󰉋  Beta | A | C\n   ~/Aliased\n   claude · filesystem\t".bytes())
            .chain(aliased.display().to_string().bytes())
            .chain([0])
            .collect::<Vec<_>>();

        assert_eq!(output, expected);
    }

    #[test]
    fn home_is_collapsed_only_in_the_display_path() {
        let directory = TestDirectory::new();
        let home = directory.0.join("home");
        let projects = [(
            home.display().to_string(),
            entry("Home", &["filesystem"], &[], false, None),
        )]
        .into_iter()
        .collect();
        let registry = write_registry(&directory.0, projects);

        let output = source_with_zoxide(&registry, &home, "", |_| Ok(None)).expect("emit source");

        assert_eq!(
            output,
            format!("󰉋  Home\n   ~\n   filesystem\t{}\0", home.display()).into_bytes()
        );
    }

    #[test]
    fn suppresses_ineligible_zoxide_fallbacks() {
        let directory = TestDirectory::new();
        let home = directory.0.join("home");
        let registered = directory.0.join("registered");
        fs::create_dir_all(&registered).expect("create registered project");
        let projects = [(
            registered
                .canonicalize()
                .expect("resolve registered project")
                .display()
                .to_string(),
            entry("Registered", &["codex"], &[], false, None),
        )]
        .into_iter()
        .collect();
        let registry = write_registry(&directory.0, projects);
        let baseline =
            source_with_zoxide(&registry, &home, "", |_| Ok(None)).expect("emit baseline");

        let mut called = false;
        let one_character = source_with_zoxide(&registry, &home, "r", |_| {
            called = true;
            Ok(Some(registered.clone()))
        })
        .expect("suppress short query");
        assert!(!called);
        assert_eq!(one_character, baseline);

        let nonexistent = source_with_zoxide(&registry, &home, "missing", |_| {
            Ok(Some(directory.0.join("missing")))
        })
        .expect("suppress nonexistent path");
        assert_eq!(nonexistent, baseline);

        let duplicate = source_with_zoxide(&registry, &home, "registered", |_| {
            Ok(Some(registered.clone()))
        })
        .expect("suppress registered path");
        assert_eq!(duplicate, baseline);
    }

    #[test]
    fn emits_an_unregistered_zoxide_directory_after_registry_rows() {
        let directory = TestDirectory::new();
        let home = directory
            .0
            .canonicalize()
            .expect("resolve test directory")
            .join("home");
        let zoxide = home.join("Zoxide");
        fs::create_dir_all(&zoxide).expect("create zoxide project");
        let registry = write_registry(&directory.0, BTreeMap::new());

        let output = source_with_zoxide(&registry, &home, "zo", |_| Ok(Some(zoxide.clone())))
            .expect("emit zoxide fallback");
        let resolved = zoxide.canonicalize().expect("resolve zoxide project");

        assert_eq!(
            output,
            format!(
                "󰉋  Zoxide\n   ~/Zoxide\n   zoxide\t{}\0",
                resolved.display()
            )
            .into_bytes()
        );
    }

    #[test]
    fn missing_registry_returns_an_error_before_emitting_output() {
        let directory = TestDirectory::new();
        let missing = directory.0.join("missing.json");

        let error = source_with_zoxide(&missing, &directory.0, "", |_| Ok(None))
            .expect_err("reject missing registry");

        assert!(error.to_string().contains("failed to read"));
    }
}
