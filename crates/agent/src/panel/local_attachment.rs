//! A permanent local-shell choice; only an explicit click may create a terminal.
use super::{
    AgentPanel,
    widgets::{menu_row, running_dot},
};
use crate::thread::{AgentThread, Attachment};
use gpui_kit::{
    AnyElement, App, Context, Entity, WeakEntity, Window,
    component::{Disableable as _, Selectable as _},
    prelude::*,
};
use nocterm_ui::IconName;
use nocterm_workspace::{TerminalEntry, TerminalStatus, Workspace};

pub(super) struct LocalAttachment;
impl LocalAttachment {
    fn terminal(thread: &AgentThread, cx: &gpui_kit::App) -> Option<TerminalEntry> {
        thread
            .workspace_seat()
            .terminals(cx)?
            .into_iter()
            .filter(|entry| {
                entry.capabilities.state().info(cx).is_some_and(|info| {
                    info.local && info.profile.is_none() && info.status != TerminalStatus::Closed
                })
            })
            .min_by_key(|entry| {
                (
                    !thread
                        .composer()
                        .attachments()
                        .contains(&Attachment::Terminal(entry.item)),
                    !entry.active,
                )
            })
    }
    pub(super) fn render(
        thread: &Entity<AgentThread>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let entry = Self::terminal(thread.read(cx), cx);
        let selected = entry.as_ref().is_some_and(|entry| {
            thread
                .read(cx)
                .composer()
                .attachments()
                .contains(&Attachment::Terminal(entry.item))
        });
        let connected = entry.as_ref().is_some_and(|entry| {
            entry
                .capabilities
                .state()
                .info(cx)
                .is_some_and(|info| info.status == TerminalStatus::Connected)
        });
        let can_create = thread.read(cx).document().key().scope()
            == nocterm_ai::history::ContextId::General
            && thread.read(cx).document().key().environment()
                == &nocterm_ai::local_environment::LocalEnvironment::ThisComputer;
        let origin = thread.entity_id();
        let original = thread.clone();
        let tooltip = if entry.is_some() {
            "Attach the local terminal"
        } else if can_create {
            "Open and attach a local terminal"
        } else {
            "Open a terminal in this workspace first"
        };
        menu_row("attach-local-terminal", "Local terminal", cx)
            .icon(IconName::Terminal)
            .selected(selected)
            .disabled(entry.is_none() && !can_create)
            .when(connected, |row| row.child(running_dot(cx)))
            .tooltip(tooltip)
            .on_click(cx.listener(move |panel, _, window, cx| {
                if let Some(entry) = &entry {
                    panel.on_attachment_selected(origin, Attachment::Terminal(entry.item), cx);
                    return;
                }
                panel.close_menus(cx);
                let panel = cx.weak_entity();
                let workspace = workspace.clone();
                let original = original.clone();
                window.defer(cx, move |window, cx| {
                    Self::create_and_attach(panel, original, workspace, window, cx);
                });
            }))
            .into_any_element()
    }

    fn create_and_attach(
        panel: WeakEntity<AgentPanel>,
        original: Entity<AgentThread>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(owner) = panel.upgrade() else { return };
        let origin = original.entity_id();
        let is_current = |cx: &App| {
            owner
                .read(cx)
                .current()
                .is_some_and(|thread| thread.entity_id() == origin)
        };
        if !is_current(cx) || original.read(cx).workspace_seat().terminals(cx).is_none() {
            return;
        }
        let _ = workspace.update(cx, |workspace, cx| workspace.new_local_terminal(window, cx));
        if !is_current(cx) {
            return;
        }
        if let Some(entry) = Self::terminal(original.read(cx), cx) {
            owner.update(cx, |panel, cx| {
                panel.on_attachment_selected(origin, Attachment::Terminal(entry.item), cx);
            });
        } else {
            owner.update(cx, |panel, cx| {
                panel.notices.set_error(Some(
                    "Could not open a local terminal. Try opening it from the terminal menu."
                        .into(),
                ));
                cx.notify();
            });
        }
    }
}
