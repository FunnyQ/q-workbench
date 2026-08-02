use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::herdr::HerdrClient;
use crate::registry::project::write_json_atomically;

const STATE_VERSION: u8 = 1;
// The single home for the harness menu labels. A stored record is keyed by the label,
// so a glyph edit in a second copy would silently invalidate every saved choice —
// `flows::agent` imports these rather than restating them.
pub const HARNESS_CLAUDE: &str = "\u{f15ce}  claude code";
pub const HARNESS_CODEX: &str = "\u{ee0d}  codex";
pub const HARNESS_OPENCODE: &str = "\u{f169f}  opencode";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAgentRecord {
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub recorded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAgentState {
    pub version: u8,
    pub panes: BTreeMap<String, LastAgentRecord>,
}

impl Default for LastAgentState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            panes: BTreeMap::new(),
        }
    }
}

fn state_path() -> Option<PathBuf> {
    let override_path = env::var_os("Q_WORKBENCH_STATE_FILE").map(PathBuf::from);
    #[cfg(test)]
    override_path.as_ref()?;
    override_path.or_else(|| {
        env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local/state/herdr-workbench/last-agent.json"))
    })
}

pub fn read_state() -> LastAgentState {
    let Some(path) = state_path() else {
        return LastAgentState::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LastAgentState>(&bytes).ok())
        .filter(|state| state.version == STATE_VERSION)
        .unwrap_or_default()
}

/// The one rule for "is this stored choice still offerable": the harness must still be
/// on the menu, and a claude record's model label must still resolve in the config.
/// Both the state reader and the harness menu ask this question, so it lives once.
pub fn last_choice_is_valid(harness: &str, model: Option<&str>, config: &Config) -> bool {
    match harness {
        HARNESS_CLAUDE => model.is_some_and(|model| {
            config
                .menu_agent()
                .is_some_and(|agent| agent.options.iter().any(|o| o.name == model))
        }),
        HARNESS_CODEX | HARNESS_OPENCODE => model.is_none(),
        _ => false,
    }
}

/// The caller already holds a loaded config, so this borrows it rather than reading and
/// parsing the file a second time on every restart.
pub fn get_for_pane(pane_id: &str, config: &Config) -> Option<(String, Option<String>)> {
    let mut state = read_state();
    let record = state.panes.get(pane_id)?.clone();
    let valid = last_choice_is_valid(&record.harness, record.model.as_deref(), config);
    if !valid {
        state.panes.remove(pane_id);
        if let Some(path) = state_path() {
            let _ = write_json_atomically(&path, &state);
        }
        return None;
    }
    Some((record.harness, record.model))
}

pub fn write_state(
    client: &dyn HerdrClient,
    pane_id: &str,
    harness: &str,
    model: Option<&str>,
) -> Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    let live_panes = client
        .pane_list(json!({}))
        .context("failed to list panes before writing agent state")?
        .panes
        .into_iter()
        .map(|pane| pane.pane_id)
        .collect::<BTreeSet<_>>();
    let mut state = read_state();
    state.panes.retain(|id, _| live_panes.contains(id));
    state.panes.insert(
        pane_id.to_owned(),
        LastAgentRecord {
            harness: harness.to_owned(),
            model: model.map(str::to_owned),
            recorded_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system clock is before Unix epoch")?
                .as_secs(),
        },
    );
    write_json_atomically(&path, &state)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::herdr::FakeClient;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn fixture(name: &str) -> PathBuf {
        env::temp_dir().join(format!("workbench-state-{name}-{}", std::process::id()))
    }

    #[test]
    fn round_trip_prunes_dead_panes_and_preserves_labels() {
        let _guard = env_lock();
        let path = fixture("round-trip");
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        fs::write(
            &path,
            r#"{"version":1,"panes":{"dead":{"harness":"old","recorded_at":1}}}"#,
        )
        .unwrap();
        let client = FakeClient::default();
        client.queue_response("pane.list", json!({"panes": [{"pane_id": "w2N:p1"}]}));

        write_state(&client, "w2N:p1", HARNESS_CODEX, None).unwrap();

        assert_eq!(
            get_for_pane("w2N:p1", &Config::test_default()),
            Some((HARNESS_CODEX.to_owned(), None))
        );
        assert!(!read_state().panes.contains_key("dead"));
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }

    #[test]
    fn missing_and_corrupt_files_are_empty() {
        let _guard = env_lock();
        let path = fixture("corrupt");
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        let _ = fs::remove_file(&path);
        assert_eq!(read_state(), LastAgentState::default());
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_state(), LastAgentState::default());
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }

    #[test]
    fn stale_harness_is_removed() {
        let _guard = env_lock();
        let path = fixture("stale");
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        fs::write(
            &path,
            r#"{"version":1,"panes":{"p1":{"harness":"removed","recorded_at":1}}}"#,
        )
        .unwrap();

        assert_eq!(get_for_pane("p1", &Config::test_default()), None);
        assert!(!read_state().panes.contains_key("p1"));
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }
}
