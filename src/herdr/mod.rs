pub mod types;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use self::types::{
    ErrorResponse, OkResponse, PaneLayoutResponse, PaneListResponse, PaneNeighborResponse,
    PaneProcessInfoResponse, PaneResponse, PingResponse, Request, SessionSnapshotResponse,
    TabCreateResponse, WorkspaceCreateResponse, WorkspaceListResponse,
};

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

    fn pane_process_info(&self, params: Value) -> Result<PaneProcessInfoResponse> {
        decode(self.call("pane.process_info", params), "pane.process_info")
    }

    fn pane_neighbor(&self, params: Value) -> Result<PaneNeighborResponse> {
        decode(self.call("pane.neighbor", params), "pane.neighbor")
    }

    fn pane_focus(&self, params: Value) -> Result<OkResponse> {
        decode(self.call("pane.focus", params), "pane.focus")
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
            return Err(anyhow!("Herdr error {}: {}", error.code, error.message));
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
            Err(anyhow!("Herdr error {}: {}", error.code, error.message))
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
}
