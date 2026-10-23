pub mod agent;
pub mod dashboard;
pub mod fork;
pub mod layout;
pub(crate) mod menu;
pub mod picker;
pub mod restart;
pub mod session;
pub mod ssh;
pub mod tab;

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use serde_json::json;

use crate::herdr::HerdrClient;

pub fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The two obvious sources are both wrong inside a plugin popup. `COLUMNS` and `LINES`
/// are shell variables that zsh does not export, so this process never sees them. `tput`
/// answers from terminfo when it holds no terminal of its own, and `Command::output()`
/// pipes both of its streams — so it reports the `xterm-256color` default of 80x24 no
/// matter how large the pane is. Measured in a 114x35 pty: `tput` said 80x24, this said
/// 114x35. Every stream is tried because only some of them are the pane's tty.
pub fn terminal_size() -> Option<(u16, u16)> {
    for fd in [libc::STDERR_FILENO, libc::STDOUT_FILENO, libc::STDIN_FILENO] {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        let read = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size as *mut libc::winsize) };
        if read == 0 && size.ws_col > 0 && size.ws_row > 0 {
            return Some((size.ws_col, size.ws_row));
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneCwd {
    PaneOnly,
    PreferForeground,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::FakeClient;

    #[test]
    fn the_popup_takes_the_pane_cwd_while_the_picker_prefers_the_cd_ed_foreground() {
        let pane_dir = std::env::temp_dir();
        let foreground_dir = PathBuf::from("/");
        let client = FakeClient::default();
        for _ in 0..2 {
            client.queue_response(
                "pane.get",
                json!({"pane": {
                    "pane_id": "p1",
                    "workspace_id": "w1",
                    "cwd": pane_dir,
                    "foreground_cwd": foreground_dir,
                }}),
            );
        }

        assert_eq!(
            invoking_pane_cwd(&client, None, Some("p1"), PaneCwd::PaneOnly),
            Some(pane_dir)
        );
        assert_eq!(
            invoking_pane_cwd(&client, None, Some("p1"), PaneCwd::PreferForeground),
            Some(foreground_dir)
        );
    }

    #[test]
    fn a_complete_failure_prints_its_sentence_once_since_prefix_and_cause_are_the_same() {
        let error = FlowError::complete("Picker", "Workspace 'x' was not found.");
        assert_eq!(error.prefix(), Some("Workspace 'x' was not found."));
        assert_eq!(error.chain(), "Workspace 'x' was not found.");
    }
}
