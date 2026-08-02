use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::herdr::HerdrClient;
use crate::registry::project::write_json_atomically;

// Bumped to 2 when the record moved from rendered menu labels to stable config names.
// `read_state()` filters on this, so a v1 file is discarded whole rather than deserialized
// into the new shape, where `harness` would silently land in nothing and `agent` be missing.
const STATE_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAgentRecord {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    pub layout: String,
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

pub fn last_choice_is_valid(record: &LastAgentRecord, config: &Config) -> bool {
    let Some(agent) = config.agent(&record.agent) else {
        return false;
    };
    // The option rule binds in both directions: an option stored for an agent that no longer
    // offers any is stale, and an agent that does offer options cannot be replayed without one.
    let option_ok = match &record.option {
        Some(name) => agent.option(name).is_some(),
        None => agent.options.is_empty(),
    };
    option_ok && config.layout(&record.layout).is_some()
}

/// The caller already holds a loaded config, so this borrows it rather than reading and
/// parsing the file a second time on every restart.
pub fn get_for_pane(pane_id: &str, config: &Config) -> Option<LastAgentRecord> {
    let mut state = read_state();
    let record = state.panes.get(pane_id)?.clone();
    let valid = last_choice_is_valid(&record, config);
    if !valid {
        state.panes.remove(pane_id);
        if let Some(path) = state_path() {
            let _ = write_json_atomically(&path, &state);
        }
        return None;
    }
    Some(record)
}

pub fn write_state(
    client: &dyn HerdrClient,
    pane_id: &str,
    record: &LastAgentRecord,
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
    state.panes.insert(pane_id.to_owned(), record.clone());
    write_json_atomically(&path, &state)
}

/// The one lock every test that mutates process environment takes. Tests run on threads
/// of a single process, so two harnesses with separate locks would still race.
#[cfg(test)]
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::FakeClient;

    fn fixture(name: &str) -> PathBuf {
        env::temp_dir().join(format!("workbench-state-{name}-{}", std::process::id()))
    }

    #[test]
    fn round_trip_prunes_dead_panes_and_preserves_names() {
        let _guard = env_lock();
        let path = fixture("round-trip");
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        fs::write(
            &path,
            r#"{"version":2,"panes":{"dead":{"agent":"old","layout":"agentic-coding","recorded_at":1}}}"#,
        )
        .unwrap();
        let client = FakeClient::default();
        client.queue_response("pane.list", json!({"panes": [{"pane_id": "w2N:p1"}]}));

        let record = LastAgentRecord {
            agent: "codex".to_owned(),
            option: None,
            layout: "agentic-coding".to_owned(),
            recorded_at: 42,
        };
        write_state(&client, "w2N:p1", &record).unwrap();

        assert_eq!(
            get_for_pane("w2N:p1", &Config::test_default()),
            Some(LastAgentRecord {
                agent: "codex".to_owned(),
                option: None,
                layout: "agentic-coding".to_owned(),
                recorded_at: 42,
            })
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
    fn version_one_state_is_discarded() {
        let _guard = env_lock();
        let path = fixture("v1");
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        fs::write(
            &path,
            r#"{"version":1,"panes":{"p1":{"harness":"codex","recorded_at":1}}}"#,
        )
        .unwrap();

        assert_eq!(read_state(), LastAgentState::default());
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }

    fn assert_stale_record_is_removed(name: &str, record: LastAgentRecord) {
        let path = fixture(name);
        env::set_var("Q_WORKBENCH_STATE_FILE", &path);
        let state = LastAgentState {
            version: STATE_VERSION,
            panes: BTreeMap::from([("p1".to_owned(), record)]),
        };
        write_json_atomically(&path, &state).unwrap();

        assert_eq!(get_for_pane("p1", &Config::test_default()), None);
        assert!(!read_state().panes.contains_key("p1"));
        fs::remove_file(path).unwrap();
        env::remove_var("Q_WORKBENCH_STATE_FILE");
    }

    #[test]
    fn stale_agent_is_removed_and_rewritten() {
        let _guard = env_lock();
        assert_stale_record_is_removed(
            "stale-agent",
            LastAgentRecord {
                agent: "removed".to_owned(),
                option: None,
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            },
        );
    }

    #[test]
    fn stale_option_is_removed() {
        let _guard = env_lock();
        assert_stale_record_is_removed(
            "stale-option",
            LastAgentRecord {
                agent: "claude code".to_owned(),
                option: Some("Removed".to_owned()),
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            },
        );
    }

    #[test]
    fn missing_option_for_optioned_agent_is_removed() {
        let _guard = env_lock();
        assert_stale_record_is_removed(
            "missing-option",
            LastAgentRecord {
                agent: "claude code".to_owned(),
                option: None,
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            },
        );
    }

    #[test]
    fn option_for_optionless_agent_is_removed() {
        let _guard = env_lock();
        assert_stale_record_is_removed(
            "unexpected-option",
            LastAgentRecord {
                agent: "codex".to_owned(),
                option: Some("Removed".to_owned()),
                layout: "agentic-coding".to_owned(),
                recorded_at: 1,
            },
        );
    }
}
