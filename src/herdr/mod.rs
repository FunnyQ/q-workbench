pub mod types;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::{error, fmt};

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use self::types::{
    ErrorResponse, LayoutApplyResponse, LayoutExportResponse, OkResponse, PaneLayoutResponse,
    PaneListResponse, PaneNeighborResponse, PaneProcessInfoResponse, PaneResponse, PingResponse,
    Request, SessionSnapshotResponse, TabCreateResponse, WorkspaceCreateResponse,
    WorkspaceListResponse,
};

/// The oldest protocol whose request and response shapes this plugin was verified
/// against. Newer protocols are accepted: Herdr adds methods and fields far more
/// often than it removes them, so an upper bound would reject working servers.
/// Raise it when a protocol drops something the plugin reads, or — the reason for 22 — when the plugin starts needing methods older servers never had.
pub const MINIMUM_PROTOCOL: u64 = 22;

#[derive(Debug)]
pub enum ProtocolGuardError {
    Connection(anyhow::Error),
    TooOld { minimum: u64, actual: u64 },
}

impl fmt::Display for ProtocolGuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(error) => write!(formatter, "failed to ping Herdr: {error:#}"),
            Self::TooOld { minimum, actual } => write!(
                formatter,
                "Herdr protocol {actual} is older than protocol {minimum}, which this plugin needs"
            ),
        }
    }
}

impl error::Error for ProtocolGuardError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Connection(error) => Some(error.as_ref()),
            Self::TooOld { .. } => None,
        }
    }
}

pub fn check_protocol(client: &dyn HerdrClient) -> Result<(), ProtocolGuardError> {
    let response = client.ping().map_err(ProtocolGuardError::Connection)?;
    if response.protocol < MINIMUM_PROTOCOL {
        return Err(ProtocolGuardError::TooOld {
            minimum: MINIMUM_PROTOCOL,
            actual: response.protocol,
        });
    }
    Ok(())
}

pub trait HerdrClient {
    fn call(&self, method: &str, params: Value) -> Result<Value>;

    fn tab_create(&self, params: Value) -> Result<TabCreateResponse> {
        decode(self.call("tab.create", params), "tab.create")
    }

    fn pane_split(&self, params: Value) -> Result<PaneResponse> {
        decode(self.call("pane.split", params), "pane.split")
    }

    fn pane_send_input(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("pane.send_input", params), "pane.send_input")
    }

    fn pane_rename(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("pane.rename", params), "pane.rename")
    }

    fn pane_list(&self, params: Value) -> Result<PaneListResponse> {
        decode(self.call("pane.list", params), "pane.list")
    }

    fn pane_get(&self, params: Value) -> Result<PaneResponse> {
        decode(self.call("pane.get", params), "pane.get")
    }

    fn pane_current(&self, params: Value) -> Result<PaneResponse> {
        decode(self.call("pane.current", params), "pane.current")
    }

    fn pane_layout(&self, params: Value) -> Result<PaneLayoutResponse> {
        decode(self.call("pane.layout", params), "pane.layout")
    }

    fn layout_export(&self, params: Value) -> Result<LayoutExportResponse> {
        decode(self.call("layout.export", params), "layout.export")
    }

    fn layout_apply(&self, params: Value) -> Result<LayoutApplyResponse> {
        decode(self.call("layout.apply", params), "layout.apply")
    }

    fn agent_start(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("agent.start", params), "agent.start")
    }

    fn pane_report_agent_session(&self, params: Value) -> Result<OkResponse> {
        decode(
            self.call("pane.report_agent_session", params),
            "pane.report_agent_session",
        )
    }

    fn layout_set_split_ratio(&self, params: Value) -> Result<OkResponse> {
        decode(
            self.call("layout.set_split_ratio", params),
            "layout.set_split_ratio",
        )
    }

    fn pane_process_info(&self, params: Value) -> Result<PaneProcessInfoResponse> {
        decode(self.call("pane.process_info", params), "pane.process_info")
    }

    fn pane_neighbor(&self, params: Value) -> Result<PaneNeighborResponse> {
        decode(self.call("pane.neighbor", params), "pane.neighbor")
    }

    fn pane_focus(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("pane.focus", params), "pane.focus")
    }

    fn pane_focus_direction(&self, params: Value) -> Result<OkResponse> {
        decode(
            self.call("pane.focus_direction", params),
            "pane.focus_direction",
        )
    }

    fn tab_rename(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("tab.rename", params), "tab.rename")
    }

    fn tab_focus(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("tab.focus", params), "tab.focus")
    }

    fn tab_close(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("tab.close", params), "tab.close")
    }

    fn workspace_create(&self, params: Value) -> Result<WorkspaceCreateResponse> {
        decode(self.call("workspace.create", params), "workspace.create")
    }

    fn workspace_list(&self, params: Value) -> Result<WorkspaceListResponse> {
        decode(self.call("workspace.list", params), "workspace.list")
    }

    fn workspace_focus(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("workspace.focus", params), "workspace.focus")
    }

    fn session_snapshot(&self, params: Value) -> Result<SessionSnapshotResponse> {
        decode(self.call("session.snapshot", params), "session.snapshot")
    }

    fn notification_show(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("notification.show", params), "notification.show")
    }

    fn ping(&self) -> Result<PingResponse> {
        decode(self.call("ping", json!({})), "ping")
    }
}

fn decode<T: DeserializeOwned>(result: Result<Value>, method: &str) -> Result<T> {
    serde_json::from_value(result?).with_context(|| format!("invalid {method} response"))
}

#[derive(Debug)]
pub struct SocketClient {
    socket_path: PathBuf,
}

impl SocketClient {
    pub fn new() -> Result<Self> {
        let socket_path = env::var_os("HERDR_SOCKET_PATH").ok_or_else(|| {
            anyhow!(
                "HERDR_SOCKET_PATH is unset; run workbench inside a Herdr pane or plugin process"
            )
        })?;

        Ok(Self {
            socket_path: socket_path.into(),
        })
    }

    #[cfg(test)]
    fn from_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }
}

impl HerdrClient for SocketClient {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        // Herdr serves one response per connection and does not support reuse.
        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("failed to connect to {}", self.socket_path.display()))?;
        let request = Request {
            id: "1".to_owned(),
            method: method.to_owned(),
            params,
        };
        let mut request_bytes =
            serde_json::to_vec(&request).context("failed to serialize Herdr request")?;
        request_bytes.push(b'\n');
        stream
            .write_all(&request_bytes)
            .context("failed to write Herdr request")?;

        // Large responses can span reads, so parsing waits for the first newline.
        let mut response = Vec::new();
        let line_end = loop {
            let mut chunk = [0_u8; 4096];
            let bytes_read = stream
                .read(&mut chunk)
                .context("failed to read Herdr response")?;
            if bytes_read == 0 {
                return Err(anyhow!(
                    "Herdr closed the socket before a complete response"
                ));
            }
            response.extend_from_slice(&chunk[..bytes_read]);
            if let Some(index) = response.iter().position(|byte| *byte == b'\n') {
                break index;
            }
        };

        let envelope: Value = serde_json::from_slice(&response[..line_end])
            .context("failed to parse Herdr response")?;
        if let Some(error) = envelope.get("error") {
            let error: ErrorResponse =
                serde_json::from_value(error.clone()).context("invalid Herdr error response")?;
            return Err(error.into());
        }

        envelope
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("Herdr response contains neither result nor error"))
    }
}

#[derive(Default)]
pub struct FakeClient {
    pub calls: RefCell<Vec<(String, Value)>>,
    /// Per-method FIFO of canned responses, consumed in call order.
    pub responses: RefCell<HashMap<String, VecDeque<Value>>>,
}

impl FakeClient {
    pub fn queue_response(&self, method: &str, response: Value) {
        self.responses
            .borrow_mut()
            .entry(method.to_owned())
            .or_default()
            .push_back(response);
    }

    pub fn queue_error(&self, method: &str, code: &str, message: &str) {
        self.queue_response(method, json!({"error": {"code": code, "message": message}}));
    }
}

impl HerdrClient for FakeClient {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.calls.borrow_mut().push((method.to_owned(), params));
        let response = self
            .responses
            .borrow_mut()
            .get_mut(method)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| json!({"type": "ok"}));

        if let Some(error) = response.get("error") {
            let error: ErrorResponse =
                serde_json::from_value(error.clone()).context("invalid fake Herdr error")?;
            Err(error.into())
        } else {
            Ok(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_client_can_be_constructed_from_a_path_for_unit_tests() {
        let client = SocketClient::from_path(PathBuf::from("/tmp/herdr.sock"));
        assert_eq!(client.socket_path, PathBuf::from("/tmp/herdr.sock"));
    }

    #[test]
    fn fake_client_consumes_responses_in_order() {
        let client = FakeClient::default();
        client.queue_response("pane.split", json!({"pane": {"pane_id": "p1"}}));
        client.queue_response("pane.split", json!({"pane": {"pane_id": "p2"}}));

        assert_eq!(client.pane_split(json!({})).unwrap().pane.pane_id, "p1");
        assert_eq!(client.pane_split(json!({})).unwrap().pane.pane_id, "p2");
        assert_eq!(client.calls.borrow().len(), 2);
    }

    #[test]
    fn fake_client_supports_defaults_and_errors() {
        let client = FakeClient::default();
        assert_eq!(client.call("tab.focus", json!({})).unwrap()["type"], "ok");

        client.queue_error("tab.focus", "not_found", "missing tab");
        let error = client.call("tab.focus", json!({})).unwrap_err().to_string();
        assert!(error.contains("not_found"));
        assert!(error.contains("missing tab"));
    }

    #[test]
    fn layout_apply_decodes_the_returned_layout_description() {
        let client = FakeClient::default();
        client.queue_response(
            "layout.apply",
            json!({
                "type": "layout_apply",
                "layout": {
                    "workspace_id": "w1",
                    "tab_id": "t6",
                    "zoomed": false,
                    "focused_pane_id": "pD",
                    "root": {"type": "pane", "pane_id": "pD"}
                }
            }),
        );

        let response = client
            .layout_apply(json!({"root": {"type": "pane"}}))
            .unwrap();

        assert_eq!(response.layout.tab_id, "t6");
        assert_eq!(
            client.calls.borrow()[0],
            ("layout.apply".to_owned(), json!({"root": {"type": "pane"}}))
        );
    }

    #[test]
    fn agent_start_and_report_agent_session_use_their_protocol_method_names() {
        let client = FakeClient::default();
        client.queue_response(
            "agent.start",
            json!({"type": "agent_started", "agent": {}, "argv": ["claude"]}),
        );

        client
            .agent_start(json!({"name": "claude", "kind": "claude", "pane_id": "p1"}))
            .unwrap();
        client
            .pane_report_agent_session(
                json!({"pane_id": "p1", "source": "q.workbench", "agent": "claude"}),
            )
            .unwrap();

        let methods: Vec<String> = client
            .calls
            .borrow()
            .iter()
            .map(|(method, _)| method.clone())
            .collect();
        assert_eq!(methods, ["agent.start", "pane.report_agent_session"]);
    }

    #[test]
    fn protocol_guard_only_pings_when_the_minimum_protocol_is_met() {
        let client = FakeClient::default();
        client.queue_response(
            "ping",
            json!({
                "type": "ping",
                "version": "1.0.0",
                "protocol": MINIMUM_PROTOCOL,
            }),
        );

        check_protocol(&client).unwrap();

        assert_eq!(
            client.calls.into_inner(),
            vec![("ping".to_owned(), json!({}))]
        );
    }

    #[test]
    fn protocol_guard_accepts_a_newer_protocol() {
        let client = FakeClient::default();
        client.queue_response(
            "ping",
            json!({
                "type": "ping",
                "version": "9.0.0",
                "protocol": MINIMUM_PROTOCOL + 5,
            }),
        );

        check_protocol(&client).unwrap();
    }

    #[test]
    fn protocol_guard_reports_both_protocols_when_herdr_is_too_old() {
        let client = FakeClient::default();
        let stale = MINIMUM_PROTOCOL - 1;
        client.queue_response(
            "ping",
            json!({"type": "ping", "version": "2.0.0", "protocol": stale}),
        );

        let error = check_protocol(&client).unwrap_err();

        assert!(matches!(
            error,
            ProtocolGuardError::TooOld { minimum, actual }
                if minimum == MINIMUM_PROTOCOL && actual == stale
        ));
        assert!(error.to_string().contains(&MINIMUM_PROTOCOL.to_string()));
        assert!(error.to_string().contains(&stale.to_string()));
    }

    #[test]
    fn protocol_guard_reports_ping_failures_as_connection_errors() {
        let client = FakeClient::default();
        client.queue_error("ping", "unavailable", "socket unavailable");

        let error = check_protocol(&client).unwrap_err();

        assert!(matches!(error, ProtocolGuardError::Connection(_)));
        assert!(error.to_string().contains("failed to ping Herdr"));
        assert!(error.to_string().contains("socket unavailable"));
    }
}
