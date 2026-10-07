//! Resolve the exact user-focused terminal again at click time.
use super::SnippetsPanel;
use gpui_kit::{App, Context, EntityId, Window};
use nocterm_workspace::{TerminalAccess, TerminalInfo, TerminalStatus};
use std::rc::Rc;
pub(super) struct RunTarget {
    pub(super) info: TerminalInfo,
    id: EntityId,
    access: Rc<dyn TerminalAccess>,
}
impl SnippetsPanel {
    pub(super) fn target(&self, cx: &App) -> Result<RunTarget, String> {
        let workspace = self.workspace.upgrade().ok_or("Workspace was closed.")?;
        let workspace = workspace.read(cx);
        let id = workspace
            .active_terminal(cx)
            .ok_or("Open and focus a terminal to run this snippet.")?;
        let entry = workspace
            .terminals(cx)
            .into_iter()
            .find(|entry| entry.item == id && !entry.background)
            .ok_or("The selected terminal is unavailable.")?;
        if entry.bottom && !workspace.local_terminal_is_visible(cx) {
            return Err("Show the selected local terminal before running snippets.".into());
        }
        let info = entry
            .access
            .info(cx)
            .ok_or("The selected terminal was closed.")?;
        Ok(RunTarget {
            id: entry.item,
            access: entry.access,
            info,
        })
    }
    pub(super) fn ready(info: &TerminalInfo) -> bool {
        info.status == TerminalStatus::Connected && info.sign_in.is_none() && !info.alt_screen
    }
    pub(super) fn run(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.target(cx).and_then(|target| {
            if !Self::ready(&target.info) {
                return Err(
                    "The selected terminal is disconnected, waiting for authentication, or in an alternate screen."
                        .into(),
                );
            }
            target.access.paste_snippet(text, true, cx)?;
            let focused = self
                .workspace
                .update(cx, |workspace, cx| {
                    workspace.focus_terminal(target.id, window, cx)
                })
                .unwrap_or(false);
            if !focused {
                return Err("The snippet was sent, but its terminal is no longer visible.".into());
            }
            Ok(())
        });
        if let Err(error) = result {
            nocterm_ui::notice::error(window, cx, "snippet-run", "Could not run snippet", error);
        }
    }
}
