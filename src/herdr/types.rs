use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkResponse {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PingResponse {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: String,
    pub protocol: u64,
    #[serde(default)]
    pub capabilities: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub agent: Option<Value>,
    #[serde(default)]
    pub agent_session: Option<AgentSessionInfo>,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub rect: Option<Value>,
    #[serde(default)]
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub pane_count: Option<u64>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub agent_status: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: Option<u64>,
    #[serde(default)]
    pub tab_count: Option<u64>,
    #[serde(default)]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub worktree: Option<Value>,
    #[serde(default)]
    pub tokens: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabCreateResponse {
    #[serde(rename = "type")]
    pub kind: String,
    pub root_pane: Pane,
    pub tab: Tab,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCreateResponse {
    #[serde(rename = "type")]
    pub kind: String,
    pub workspace: Workspace,
    pub tab: Tab,
    pub root_pane: Pane,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneResponse {
    pub pane: Pane,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneListResponse {
    #[serde(default)]
    pub panes: Vec<Pane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneLayoutResponse {
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutExportResponse {
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    #[serde(default)]
    pub foreground_process_group_id: Option<i32>,
    #[serde(default)]
    pub shell_pid: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneProcessInfoResponse {
    #[serde(default)]
    pub process_info: Option<ProcessInfo>,
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Neighbor {
    #[serde(default)]
    pub neighbor_pane_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneNeighborResponse {
    #[serde(default)]
    pub neighbor: Option<Neighbor>,
    #[serde(default)]
    pub pane: Option<Pane>,
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceListResponse {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshotResponse {
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

/// One type serves both directions because `layout.apply` sends the same node
/// shape that `layout.export` returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Pane {
        // Herdr rejects nothing here, but omitting empties keeps apply requests readable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<Vec<String>>,
    },
    Split {
        direction: String,
        ratio: f64,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutDescription {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub focused_pane_id: String,
    pub root: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutApplyResponse {
    pub layout: LayoutDescription,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionInfo {
    pub source: String,
    pub agent: String,
    pub kind: String,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bare_pane_leaf_serialises_to_type_alone() {
        let leaf = LayoutNode::Pane {
            pane_id: None,
            cwd: None,
            env: BTreeMap::new(),
            label: None,
            command: None,
        };

        assert_eq!(
            serde_json::to_value(&leaf).unwrap(),
            json!({"type": "pane"})
        );
    }

    #[test]
    fn a_populated_pane_leaf_serialises_every_set_field() {
        let leaf = LayoutNode::Pane {
            pane_id: Some("p1".to_owned()),
            cwd: Some("/tmp/project".to_owned()),
            env: BTreeMap::from([("EDITOR".to_owned(), "nvim".to_owned())]),
            label: Some("agent".to_owned()),
            command: Some(vec!["yazi".to_owned(), ".".to_owned()]),
        };

        assert_eq!(
            serde_json::to_value(&leaf).unwrap(),
            json!({
                "type": "pane",
                "pane_id": "p1",
                "cwd": "/tmp/project",
                "env": {"EDITOR": "nvim"},
                "label": "agent",
                "command": ["yazi", "."],
            })
        );
    }

    #[test]
    fn a_split_node_round_trips_through_json() {
        let split = LayoutNode::Split {
            direction: "down".to_owned(),
            ratio: 0.9,
            first: Box::new(LayoutNode::Pane {
                pane_id: None,
                cwd: None,
                env: BTreeMap::new(),
                label: Some("top".to_owned()),
                command: None,
            }),
            second: Box::new(LayoutNode::Pane {
                pane_id: None,
                cwd: None,
                env: BTreeMap::new(),
                label: Some("bottom".to_owned()),
                command: None,
            }),
        };

        let encoded = serde_json::to_value(&split).unwrap();
        assert_eq!(encoded["type"], "split");
        assert_eq!(encoded["direction"], "down");
        assert_eq!(encoded["ratio"], 0.9);
        assert_eq!(
            serde_json::from_value::<LayoutNode>(encoded).unwrap(),
            split
        );
    }

    #[test]
    fn a_layout_apply_response_parses_the_shared_layout_description() {
        let response: LayoutApplyResponse = serde_json::from_value(json!({
            "type": "layout_apply",
            "layout": {
                "workspace_id": "w1",
                "tab_id": "t6",
                "zoomed": false,
                "focused_pane_id": "pD",
                "root": {
                    "type": "split",
                    "direction": "right",
                    "ratio": 0.7,
                    "first": {"type": "pane", "pane_id": "pD", "label": "agent"},
                    "second": {"type": "pane", "pane_id": "pE"}
                }
            }
        }))
        .unwrap();

        assert_eq!(response.layout.tab_id, "t6");
        assert_eq!(response.layout.focused_pane_id, "pD");
        let LayoutNode::Split { first, ratio, .. } = response.layout.root else {
            panic!("expected a split root");
        };
        assert!((ratio - 0.7).abs() < 1e-9);
        assert_eq!(
            *first,
            LayoutNode::Pane {
                pane_id: Some("pD".to_owned()),
                cwd: None,
                env: BTreeMap::new(),
                label: Some("agent".to_owned()),
                command: None,
            }
        );
    }

    #[test]
    fn a_pane_carries_its_reported_agent_session() {
        let pane: Pane = serde_json::from_value(json!({
            "pane_id": "p1",
            "agent_session": {
                "source": "q.workbench",
                "agent": "claude",
                "kind": "id",
                "value": "abc-123"
            }
        }))
        .unwrap();

        assert_eq!(
            pane.agent_session,
            Some(AgentSessionInfo {
                source: "q.workbench".to_owned(),
                agent: "claude".to_owned(),
                kind: "id".to_owned(),
                value: "abc-123".to_owned(),
            })
        );
    }

    #[test]
    fn a_pane_without_an_agent_session_still_parses() {
        let pane: Pane = serde_json::from_value(json!({"pane_id": "p1"})).unwrap();

        assert_eq!(pane.agent_session, None);
    }
}
