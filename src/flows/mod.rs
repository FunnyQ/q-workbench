pub mod agent;
pub mod dashboard;
pub mod picker;
pub mod restart;
pub mod ssh;

use std::error::Error;
use std::fmt;

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
