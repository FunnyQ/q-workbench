use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::flows::restart::detached_command;
use crate::flows::{FlowResult, Outcome};
use crate::herdr::HerdrClient;
use crate::registry::project::first_line;
use crate::state;

/// Who Herdr credits the reported session to, since this plugin resolves it rather than
/// Herdr's own integration hooks.
const SOURCE: &str = "q.workbench";
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const POLL_ATTEMPTS: u32 = 60;

/// A session file this pane's agent is believed to have written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSession {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportOptions {
    pub pane_id: String,
    pub agent: String,
    pub kind: String,
    pub cwd: PathBuf,
    pub since_ms: u64,
}

/// A heuristic: two agents launched into one cwd within the same window — which the `pair`
/// layout does — can still be credited each other's session. Creation time is what keeps an
/// agent that was already running there out of the answer.
pub fn resolve_session(
    kind: &str,
    home: &Path,
    cwd: &Path,
    since_ms: u64,
) -> Option<ResolvedSession> {
    match kind {
        "claude" => claude_session(home, cwd, since_ms),
        "codex" => codex_session(home, cwd, since_ms),
        _ => None,
    }
}

/// Runs detached, so a pane whose agent never writes a session file exits without a word.
pub fn report(client: &dyn HerdrClient, options: &ReportOptions) -> FlowResult {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Ok(Outcome::Cancelled);
    };
    for attempt in 0..POLL_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(POLL_INTERVAL);
        }
        let Some(session) = resolve_session(&options.kind, &home, &options.cwd, options.since_ms)
        else {
            continue;
        };
        client
            .pane_report_agent_session(json!({
                "pane_id": options.pane_id,
                "source": SOURCE,
                "agent": options.agent,
                "agent_session_id": session.id,
                "agent_session_path": session.path,
            }))
            .context("failed to report the agent session")?;
        state::set_session(&options.pane_id, &session.id)
            .context("failed to store the agent session")?;
        return Ok(Outcome::Done);
    }
    Ok(Outcome::Cancelled)
}

/// Starts the reporter detached, because the caller is about to `exec` itself away.
pub fn spawn_reporter(options: &ReportOptions) -> Result<()> {
    let executable =
        std::env::current_exe().context("failed to resolve the workbench executable")?;
    detached_command(&executable, &reporter_argv(options))
        .spawn()
        .context("failed to spawn the session reporter")?;
    Ok(())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn reporter_argv(options: &ReportOptions) -> Vec<String> {
    vec![
        "agent".to_owned(),
        "session-report".to_owned(),
        "--pane".to_owned(),
        options.pane_id.clone(),
        "--agent".to_owned(),
        options.agent.clone(),
        "--kind".to_owned(),
        options.kind.clone(),
        "--cwd".to_owned(),
        options.cwd.to_string_lossy().into_owned(),
        "--since".to_owned(),
        options.since_ms.to_string(),
    ]
}

fn claude_session(home: &Path, cwd: &Path, since_ms: u64) -> Option<ResolvedSession> {
    let directory = home.join(".claude/projects").join(claude_slug(cwd));
    // Flat, not recursive: `subagents/` holds transcripts that are not the pane's session.
    let path = files_newer_than(&directory, since_ms, false, &|path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    })
    .into_iter()
    .next()?;
    let id = path.file_stem()?.to_str()?.to_owned();
    Some(ResolvedSession { id, path })
}

fn codex_session(home: &Path, cwd: &Path, since_ms: u64) -> Option<ResolvedSession> {
    let sessions = home.join(".codex/sessions");
    let cwd = cwd.to_str()?;
    for path in files_newer_than(&sessions, since_ms, true, &|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
    }) {
        // Every project's rollouts share one tree, so the header's cwd is what picks ours.
        let Some(meta) = first_line(&path).and_then(|line| line.parse::<Value>().ok()) else {
            continue;
        };
        if meta.pointer("/payload/cwd").and_then(Value::as_str) != Some(cwd) {
            continue;
        }
        if let Some(id) = meta.pointer("/payload/id").and_then(Value::as_str) {
            return Some(ResolvedSession {
                id: id.to_owned(),
                path,
            });
        }
    }
    None
}

/// Claude Code's project-directory encoding: every character outside `[A-Za-z0-9-]`
/// becomes a dash, so `/Users/q/.claude` is stored as `-Users-q--claude`.
fn claude_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// Matching files created after `since_ms`, newest first. The stamp is read before any
/// file is opened, which is what keeps a poll over thousands of rollouts affordable.
fn files_newer_than(
    directory: &Path,
    since_ms: u64,
    recursive: bool,
    matches: &dyn Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_newer(directory, since_ms, recursive, matches, &mut found);
    found.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    found.into_iter().map(|(path, _)| path).collect()
}

fn collect_newer(
    directory: &Path,
    since_ms: u64,
    recursive: bool,
    matches: &dyn Fn(&Path) -> bool,
    found: &mut Vec<(PathBuf, u64)>,
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
            if recursive {
                collect_newer(&path, since_ms, recursive, matches, found);
            }
            continue;
        }
        if !file_type.is_file() || !matches(&path) {
            continue;
        }
        // Created, not modified: a session already running in this cwd keeps its transcript
        // newer than any launch stamp, so modification time claims it for every new pane.
        let Some(created) = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.created().ok())
            .and_then(|created| created.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64)
        else {
            continue;
        };
        if created > since_ms {
            found.push((path, created));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, FileTimes};
    use std::os::macos::fs::FileTimesExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::herdr::FakeClient;

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        home: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let home = std::env::temp_dir().join(format!(
                "workbench-session-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&home).unwrap();
            Self { home }
        }

        /// Writes a file and pins its mtime, so "newer than `--since`" is asserted
        /// against a fixed clock rather than however fast the test ran.
        /// Stamps creation, which is what the sweep reads, and modification with it so a
        /// fixture cannot pass by accident on the wrong one.
        fn write_at(&self, path: &str, contents: &str, created_ms: u64) {
            self.write_stamped(path, contents, created_ms, created_ms);
        }

        fn write_stamped(&self, path: &str, contents: &str, created_ms: u64, modified_ms: u64) {
            let path = self.home.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            let file = File::options().write(true).open(&path).unwrap();
            file.set_times(
                FileTimes::new()
                    .set_created(UNIX_EPOCH + Duration::from_millis(created_ms))
                    .set_modified(UNIX_EPOCH + Duration::from_millis(modified_ms)),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn codex_rollout(id: &str, cwd: &str) -> String {
        json!({
            "timestamp": "2026-09-09T11:16:03.000Z",
            "type": "session_meta",
            "payload": {"id": id, "cwd": cwd},
        })
        .to_string()
    }

    /// An agent already running in this cwd keeps writing, so modification time made its
    /// live session the newest file and handed it to every new pane.
    #[test]
    fn a_session_already_running_in_this_cwd_is_not_claimed() {
        let fixture = Fixture::new("claude-running");
        let slug = "-Users-q-Projects-demo";
        // Started long before this pane, still being written to.
        fixture.write_stamped(
            &format!(".claude/projects/{slug}/live-elsewhere.jsonl"),
            "{}",
            1_000,
            9_000,
        );
        fixture.write_at(&format!(".claude/projects/{slug}/ours.jsonl"), "{}", 5_000);

        let resolved = resolve_session(
            "claude",
            &fixture.home,
            Path::new("/Users/q/Projects/demo"),
            4_000,
        )
        .unwrap();

        assert_eq!(resolved.id, "ours");
    }

    #[test]
    fn only_a_claude_transcript_newer_than_since_is_taken() {
        let fixture = Fixture::new("claude-since");
        let slug = "-Users-q-Projects-demo";
        fixture.write_at(&format!(".claude/projects/{slug}/older.jsonl"), "{}", 1_000);
        fixture.write_at(&format!(".claude/projects/{slug}/newer.jsonl"), "{}", 3_000);

        let resolved = resolve_session(
            "claude",
            &fixture.home,
            Path::new("/Users/q/Projects/demo"),
            2_000,
        )
        .unwrap();

        // The transcript is named after the session, so the stem is the id.
        assert_eq!(resolved.id, "newer");
        assert_eq!(
            resolved.path,
            fixture
                .home
                .join(format!(".claude/projects/{slug}/newer.jsonl"))
        );

        // Push the window past both and nothing is claimed.
        assert_eq!(
            resolve_session(
                "claude",
                &fixture.home,
                Path::new("/Users/q/Projects/demo"),
                4_000,
            ),
            None
        );
    }

    #[test]
    fn claude_ignores_nested_transcripts_and_non_transcripts() {
        let fixture = Fixture::new("claude-nested");
        let slug = "-Users-q--config-demo";
        // Subagent transcripts live one level down and are not the pane's session.
        fixture.write_at(
            &format!(".claude/projects/{slug}/subagents/nested.jsonl"),
            "{}",
            9_000,
        );
        // Third-party sidecars sit beside the transcript and share its stem.
        fixture.write_at(
            &format!(".claude/projects/{slug}/real.jsonl.wakatime"),
            "{}",
            9_000,
        );
        fixture.write_at(&format!(".claude/projects/{slug}/real.jsonl"), "{}", 3_000);

        let resolved = resolve_session(
            "claude",
            &fixture.home,
            Path::new("/Users/q/.config/demo"),
            2_000,
        )
        .unwrap();

        assert_eq!(resolved.id, "real");
    }

    #[test]
    fn a_codex_rollout_is_matched_by_the_cwd_in_its_header() {
        let fixture = Fixture::new("codex-cwd");
        fixture.write_at(
            ".codex/sessions/2026/09/09/rollout-2026-09-09T11-00-00-aaa.jsonl",
            &codex_rollout("aaa", "/Users/q/Projects/demo"),
            3_000,
        );
        // Newer, but another project's session.
        fixture.write_at(
            ".codex/sessions/2026/09/09/rollout-2026-09-09T11-10-00-bbb.jsonl",
            &codex_rollout("bbb", "/Users/q/Projects/other"),
            5_000,
        );
        // Ours, but from before the launch.
        fixture.write_at(
            ".codex/sessions/2026/09/08/rollout-2026-09-08T11-00-00-ccc.jsonl",
            &codex_rollout("ccc", "/Users/q/Projects/demo"),
            1_000,
        );

        let resolved = resolve_session(
            "codex",
            &fixture.home,
            Path::new("/Users/q/Projects/demo"),
            2_000,
        )
        .unwrap();

        assert_eq!(resolved.id, "aaa");
    }

    #[test]
    fn a_kind_with_no_known_session_layout_resolves_nothing() {
        let fixture = Fixture::new("unknown-kind");
        assert_eq!(
            resolve_session("opencode", &fixture.home, Path::new("/Users/q"), 0),
            None
        );
    }

    #[test]
    fn a_resolved_session_is_reported_to_herdr_and_stored_in_state() {
        let _guard = state::env_lock();
        let fixture = Fixture::new("report");
        let slug = "-Users-q-Projects-demo";
        fixture.write_at(&format!(".claude/projects/{slug}/s-1.jsonl"), "{}", 3_000);

        let state_file = fixture.home.join("last-agent.json");
        std::env::set_var("Q_WORKBENCH_STATE_FILE", &state_file);
        let home = std::env::var_os("HOME");
        std::env::set_var("HOME", &fixture.home);
        let client = FakeClient::default();
        client.queue_response("pane.list", json!({"panes": [{"pane_id": "w1:p1"}]}));
        state::write_state(
            &client,
            "w1:p1",
            &state::LastAgentRecord {
                agent: "claude code".to_owned(),
                option: None,
                effort: None,
                layout: "agentic-coding".to_owned(),
                pane: "agent".to_owned(),
                session: None,
                recorded_at: 1,
            },
        )
        .unwrap();

        let outcome = report(
            &client,
            &ReportOptions {
                pane_id: "w1:p1".to_owned(),
                agent: "claude code".to_owned(),
                kind: "claude".to_owned(),
                cwd: PathBuf::from("/Users/q/Projects/demo"),
                since_ms: 2_000,
            },
        )
        .unwrap();

        assert_eq!(outcome, Outcome::Done);
        let calls = client.calls.borrow();
        let (method, params) = calls.last().unwrap();
        assert_eq!(method, "pane.report_agent_session");
        assert_eq!(params["pane_id"], "w1:p1");
        assert_eq!(params["source"], "q.workbench");
        assert_eq!(params["agent"], "claude code");
        assert_eq!(params["agent_session_id"], "s-1");
        assert_eq!(
            params["agent_session_path"],
            fixture
                .home
                .join(format!(".claude/projects/{slug}/s-1.jsonl"))
                .to_str()
                .unwrap()
        );
        assert_eq!(
            state::read_state().panes["w1:p1"].session.as_deref(),
            Some("s-1")
        );

        std::env::remove_var("Q_WORKBENCH_STATE_FILE");
        match home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn the_reporter_argv_carries_every_field_the_worker_needs() {
        assert_eq!(
            reporter_argv(&ReportOptions {
                pane_id: "w1:p1".to_owned(),
                agent: "claude code".to_owned(),
                kind: "claude".to_owned(),
                cwd: PathBuf::from("/Users/q/Projects/demo"),
                since_ms: 1_757_416_563_000,
            }),
            [
                "agent",
                "session-report",
                "--pane",
                "w1:p1",
                "--agent",
                "claude code",
                "--kind",
                "claude",
                "--cwd",
                "/Users/q/Projects/demo",
                "--since",
                "1757416563000",
            ]
        );
    }
}
