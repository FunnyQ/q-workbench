use anyhow::{anyhow, Context};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::herdr::HerdrClient;

use super::{nonempty_env, FlowResult, Outcome};

/// The two node shapes `layout.export` returns for a tab's split tree.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LayoutNode {
    Pane {
        pane_id: String,
    },
    Split {
        direction: String,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

/// Set every split ratio in the target pane's row (or column) so its siblings end up
/// the same size. Only splits that share the immediate parent's direction are touched.
/// A pane split in a different direction nested inside one slot — a Files/term stack
/// inside one column of a row, say — is left alone: evening it out would resize a
/// dimension the caller never asked about.
pub fn even_out(client: &dyn HerdrClient, pane: Option<&str>) -> FlowResult {
    let pane_id = pane
        .map(str::to_owned)
        .or_else(|| nonempty_env("HERDR_PANE_ID"))
        .or_else(|| nonempty_env("HERDR_ACTIVE_PANE_ID"))
        .context("no target pane: pass --pane or run inside a Herdr pane")?;

    let export = client
        .layout_export(json!({ "pane_id": pane_id }))
        .context("failed to read the tab layout")?;
    let layout = export
        .fields
        .get("layout")
        .context("layout.export returned no layout")?;
    let tab_id = layout
        .get("tab_id")
        .and_then(Value::as_str)
        .context("layout.export returned no tab_id")?
        .to_owned();
    let root: LayoutNode = serde_json::from_value(
        layout
            .get("root")
            .context("layout.export returned no root")?
            .clone(),
    )
    .context("failed to parse the tab's split layout")?;

    let path = find_path(&root, &pane_id)
        .ok_or_else(|| anyhow!("pane {pane_id} was not found in its own tab's layout"))?;
    if path.is_empty() {
        return Ok(Outcome::Notice {
            title: "Even out panes".into(),
            body: "This pane has no siblings to even out.".into(),
        });
    }

    let direction = direction_at(&root, &path[..path.len() - 1])
        .context("failed to resolve the pane's split direction")?;
    let mut chain_depth = path.len() - 1;
    while chain_depth > 0 && direction_at(&root, &path[..chain_depth - 1]) == Some(direction) {
        chain_depth -= 1;
    }
    let chain_root =
        node_at(&root, &path[..chain_depth]).context("failed to resolve the row's root split")?;

    apply_even_ratios(client, &tab_id, direction, chain_root, &path[..chain_depth])?;
    Ok(Outcome::Done)
}

/// The branch choices (`false` = first, `true` = second) from `root` down to the pane,
/// or `None` when the pane is not in this tree.
fn find_path(node: &LayoutNode, pane_id: &str) -> Option<Vec<bool>> {
    match node {
        LayoutNode::Pane { pane_id: id } => (id == pane_id).then(Vec::new),
        LayoutNode::Split { first, second, .. } => {
            if let Some(mut path) = find_path(first, pane_id) {
                path.insert(0, false);
                return Some(path);
            }
            let mut path = find_path(second, pane_id)?;
            path.insert(0, true);
            Some(path)
        }
    }
}

fn node_at<'a>(root: &'a LayoutNode, path: &[bool]) -> Option<&'a LayoutNode> {
    path.iter().try_fold(root, |node, &branch| match node {
        LayoutNode::Split { first, second, .. } => {
            Some(if branch { second } else { first }.as_ref())
        }
        LayoutNode::Pane { .. } => None,
    })
}

fn direction_at<'a>(root: &'a LayoutNode, path: &[bool]) -> Option<&'a str> {
    match node_at(root, path)? {
        LayoutNode::Split { direction, .. } => Some(direction.as_str()),
        LayoutNode::Pane { .. } => None,
    }
}

/// Even out `node`'s split ratios in-place over the socket, returning how many leaf
/// panes it holds. A split in a different direction — an orthogonal sub-layout nested
/// inside one slot — counts as a single opaque unit and is left untouched.
fn apply_even_ratios(
    client: &dyn HerdrClient,
    tab_id: &str,
    direction: &str,
    node: &LayoutNode,
    path: &[bool],
) -> anyhow::Result<u64> {
    let LayoutNode::Split {
        direction: node_direction,
        first,
        second,
    } = node
    else {
        return Ok(1);
    };
    if node_direction != direction {
        return Ok(1);
    }

    let mut first_path = path.to_vec();
    first_path.push(false);
    let first_count = apply_even_ratios(client, tab_id, direction, first, &first_path)?;

    let mut second_path = path.to_vec();
    second_path.push(true);
    let second_count = apply_even_ratios(client, tab_id, direction, second, &second_path)?;

    let ratio = first_count as f64 / (first_count + second_count) as f64;
    client
        .layout_set_split_ratio(json!({ "tab_id": tab_id, "path": path, "ratio": ratio }))
        .with_context(|| format!("failed to set split ratio at {path:?}"))?;

    Ok(first_count + second_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::FakeClient;

    /// The exact tree Herdr returned for a pane split right, then split right again:
    /// a 50/25/25 layout from two naive 50/50 splits (the shape in the bug report).
    fn three_pane_row() -> Value {
        json!({
            "layout": {
                "tab_id": "t1",
                "root": {
                    "type": "split",
                    "direction": "right",
                    "ratio": 0.5,
                    "first": { "type": "pane", "pane_id": "p1" },
                    "second": {
                        "type": "split",
                        "direction": "right",
                        "ratio": 0.5,
                        "first": { "type": "pane", "pane_id": "p2" },
                        "second": { "type": "pane", "pane_id": "p3" }
                    }
                }
            }
        })
    }

    #[test]
    fn evens_out_a_three_pane_row_with_thirds() {
        let client = FakeClient::default();
        client.queue_response("layout.export", three_pane_row());

        let outcome = even_out(&client, Some("p3")).unwrap();

        assert_eq!(outcome, Outcome::Done);
        let calls = client.calls.borrow();
        let ratio_calls: Vec<&(String, Value)> = calls
            .iter()
            .filter(|(method, _)| method == "layout.set_split_ratio")
            .collect();
        assert_eq!(ratio_calls.len(), 2);
        assert_eq!(ratio_calls[0].1["tab_id"], "t1");
        assert_eq!(ratio_calls[0].1["path"], json!([true]));
        assert!((ratio_calls[0].1["ratio"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(ratio_calls[1].1["path"], json!([]));
        assert!((ratio_calls[1].1["ratio"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn leaves_an_orthogonal_split_untouched() {
        let client = FakeClient::default();
        client.queue_response(
            "layout.export",
            json!({
                "layout": {
                    "tab_id": "t1",
                    "root": {
                        "type": "split",
                        "direction": "right",
                        "ratio": 0.5,
                        "first": { "type": "pane", "pane_id": "p1" },
                        "second": {
                            "type": "split",
                            "direction": "down",
                            "ratio": 0.9,
                            "first": { "type": "pane", "pane_id": "p2" },
                            "second": { "type": "pane", "pane_id": "p3" }
                        }
                    }
                }
            }),
        );

        let outcome = even_out(&client, Some("p2")).unwrap();

        assert_eq!(outcome, Outcome::Done);
        let calls = client.calls.borrow();
        let ratio_calls: Vec<&(String, Value)> = calls
            .iter()
            .filter(|(method, _)| method == "layout.set_split_ratio")
            .collect();
        assert_eq!(ratio_calls.len(), 1);
        assert_eq!(ratio_calls[0].1["path"], json!([true]));
        assert!((ratio_calls[0].1["ratio"].as_f64().unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn does_nothing_for_a_lone_pane() {
        let client = FakeClient::default();
        client.queue_response(
            "layout.export",
            json!({
                "layout": {
                    "tab_id": "t1",
                    "root": { "type": "pane", "pane_id": "p1" }
                }
            }),
        );

        let outcome = even_out(&client, Some("p1")).unwrap();

        assert!(matches!(outcome, Outcome::Notice { .. }));
        let calls = client.calls.borrow();
        assert!(!calls
            .iter()
            .any(|(method, _)| method == "layout.set_split_ratio"));
    }
}
