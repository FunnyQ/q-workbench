pub mod agent;
pub mod dashboard;
pub mod picker;
pub mod restart;
pub mod ssh;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde_json::json;

use crate::herdr::HerdrClient;

/// Which cwd of the invoking pane a flow adopts.
///
/// The two entry points genuinely differ and always have. The agent popup wants the
/// pane's own directory, because that is the repository the launcher runs git against;
/// the project picker prefers the foreground process's directory, so a shell the user
/// has `cd`-ed inside still seeds the right query. Naming the difference here is the
/// point: written as two helpers it read as drift, and either one could be "corrected"
/// into the other by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneCwd {
    PaneOnly,
    PreferForeground,
}

/// The cwd of the pane that invoked this plugin action, or `None` when nothing usable
/// is reachable. `HERDR_PLUGIN_CONTEXT_JSON` answers without a round trip; the
/// `HERDR_ACTIVE_PANE_ID` lookup is the fallback for a caller that has no context.
pub fn invoking_pane_cwd(
    client: &dyn HerdrClient,
    context_json: Option<&str>,
    active_pane_id: Option<&str>,
    which: PaneCwd,
) -> Option<PathBuf> {
    let context_cwd = context_json
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value.get("focused_pane_cwd")?.as_str().map(PathBuf::from))
        .filter(|path| path.is_dir());
    if context_cwd.is_some() {
        return context_cwd;
    }
    let pane = client
        .pane_get(json!({ "pane_id": active_pane_id? }))
        .ok()?
        .pane;
    match which {
        PaneCwd::PaneOnly => pane.cwd,
        PaneCwd::PreferForeground => pane.foreground_cwd.or(pane.cwd),
    }
    .map(PathBuf::from)
    .filter(|path| path.is_dir())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled,
    Notice { title: String, body: String },
}

pub type FlowResult = anyhow::Result<Outcome>;

#[derive(Debug)]
pub struct FlowError {
    title: Option<String>,
    prefix: Option<String>,
    error: anyhow::Error,
}

impl FlowError {
    pub fn titled(title: impl Into<String>, error: anyhow::Error) -> Self {
        Self {
            title: Some(title.into()),
            prefix: None,
            error,
        }
    }

    pub fn prefixed(
        title: impl Into<String>,
        prefix: impl Into<String>,
        error: anyhow::Error,
    ) -> Self {
        Self {
            title: Some(title.into()),
            prefix: Some(prefix.into()),
            error,
        }
    }

    /// A failure whose sentence is already the complete notification body.
    ///
    /// This adds no field: the sentence is stored as both the preserved prefix and the
    /// error itself, so the chained cause the reporting path would append is that same
    /// sentence and it prints once. Use it when the message already names its concrete
    /// cause — `Workspace 'x' was not found.`, `project picker: registry not found: …`
    /// — and `prefixed` when the sentence describes something else, such as the agent
    /// popup's `The incomplete tab was closed.`, which needs the cause after it.
    pub fn complete(title: impl Into<String>, body: impl Into<String>) -> Self {
        let body = body.into();
        Self::prefixed(title, body.clone(), anyhow::anyhow!(body))
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    pub fn chain(&self) -> String {
        format!("{:#}", self.error)
    }
}

impl fmt::Display for FlowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:#}", self.error)
    }
}

impl Error for FlowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.error.as_ref())
    }
}
