//! Resolve the exact user-focused terminal again at click time.
use super::SnippetsPanel;
use gpui_kit::{App, Context, EntityId, Window};
use nocterm_workspace::{TerminalInfo, TerminalStatus, UserTerminalPaste};
use std::rc::Rc;
pub(super) struct RunTarget {
    info: TerminalInfo,
    id: EntityId,
    paste: Rc<dyn UserTerminalPaste>,
}
impl RunTarget {
    pub(super) fn into_info(self) -> TerminalInfo {
        self.info
    }
}
#[derive(Debug, thiserror::Error)]
pub(super) enum TargetError {
    #[error("{0}")]
    Terminal(#[from] nocterm_workspace::TerminalError),
    #[error("Workspace was closed.")]
    WorkspaceClosed,
    #[error("Open and focus a terminal to run this snippet.")]
    NoActiveTerminal,
    #[error("The selected terminal is unavailable.")]
    Unavailable,
    #[error("Show the selected local terminal before running snippets.")]
    LocalTerminalHidden,
    #[error("The selected terminal was closed.")]
    TerminalClosed,
}

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("{0}")]
    Target(#[from] TargetError),
    #[error("{0}")]
    Terminal(#[from] nocterm_workspace::TerminalError),
    #[error(
        "The selected terminal is disconnected, waiting for authentication, or in an alternate screen."
    )]
    NotReady,
    #[error("The snippet was sent, but its terminal is no longer visible.")]
    SentButNoLongerVisible,
}

impl SnippetsPanel {
    pub(super) fn target(&self, cx: &App) -> Result<RunTarget, TargetError> {
        let workspace = self
            .workspace
            .upgrade()
            .ok_or(TargetError::WorkspaceClosed)?;
        let workspace = workspace.read(cx);
        let id = workspace
            .active_terminal(cx)
            .ok_or(TargetError::NoActiveTerminal)?;
        let entry = workspace
            .terminals(cx)
            .into_iter()
            .find(|entry| entry.item == id && !entry.background)
            .ok_or(TargetError::Unavailable)?;
        if entry.bottom && !workspace.local_terminal_is_visible(cx) {
            return Err(TargetError::LocalTerminalHidden);
        }
        let info = entry
            .capabilities
            .state()
            .info(cx)
            .ok_or(TargetError::TerminalClosed)?;
        Ok(RunTarget {
            id: entry.item,
            paste: entry
                .capabilities
                .paste()
                .cloned()
                .ok_or(TargetError::Terminal(
                    nocterm_workspace::TerminalError::Unsupported(
                        nocterm_workspace::TerminalCapability::Snippets,
                    ),
                ))?,
            info,
        })
    }
    pub(super) fn ready(info: &TerminalInfo) -> bool {
        info.status == TerminalStatus::Connected && info.sign_in.is_none() && !info.alt_screen
    }
    pub(super) fn run(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<(), RunError> {
            let target = self.target(cx)?;
            if !Self::ready(&target.info) {
                return Err(RunError::NotReady);
            }
            target.paste.paste(target.info.expected, text, true, cx)?;
            let focused = self
                .workspace
                .update(cx, |workspace, cx| {
                    workspace.focus_terminal(target.id, window, cx)
                })
                .unwrap_or(false);
            if !focused {
                return Err(RunError::SentButNoLongerVisible);
            }
            Ok(())
        })();
        if let Err(error) = result {
            nocterm_ui::notice::error(
                window,
                cx,
                "snippet-run",
                "Could not run snippet",
                error.to_string(),
            );
        }
    }
}
