//! An immutable original surface and exact document, independent of visible tabs.
use gpui_kit::{App, EntityId, Subscription, WeakEntity, Window};
use nocterm_agent_chat::seat::{ChatWorkspace, Seat};
use nocterm_ai::{
    execution_origin::ChatKey,
    history::ContextId,
    storage::{AgentStorageError, require_chat_scope},
};
use nocterm_workspace::{
    ConnectionSummary, ExpectedSession, ProjectSurfaceReceipt, TerminalEntry, Workspace,
};
use std::rc::Rc;

pub(crate) struct WorkspaceSeat {
    workspace: WeakEntity<Workspace>,
    source: ProjectSurfaceReceipt,
    key: Option<ChatKey>,
}
impl WorkspaceSeat {
    pub(crate) fn new(workspace: WeakEntity<Workspace>, source: ProjectSurfaceReceipt) -> Self {
        Self {
            workspace,
            source,
            key: None,
        }
    }
    fn permits(&self, workspace: &Workspace, item: EntityId, cx: &App) -> bool {
        workspace
            .context_terminals(self.source, cx)
            .is_ok_and(|entries| entries.iter().any(|entry| entry.item == item))
    }
}
impl ChatWorkspace for WorkspaceSeat {
    fn for_chat(&self, key: &ChatKey) -> Result<Seat, AgentStorageError> {
        require_chat_scope(self.source.context(), key.scope())?;
        if self.key.as_ref().is_some_and(|original| original != key) {
            return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
        }
        Ok(Rc::new(Self {
            workspace: self.workspace.clone(),
            source: self.source,
            key: Some(key.clone()),
        }))
    }
    fn id(&self) -> EntityId {
        self.workspace.entity_id()
    }
    fn is_open(&self) -> bool {
        self.workspace.upgrade().is_some()
    }
    fn terminals(&self, cx: &App) -> Option<Vec<TerminalEntry>> {
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        workspace.context_terminals(self.source, cx).ok()
    }
    fn connections(&self, cx: &App) -> Vec<ConnectionSummary> {
        if self.terminals(cx).is_none() {
            return Vec::new();
        }
        self.workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).connection_directory())
            .map(|directory| directory.connections(cx))
            .unwrap_or_default()
    }
    fn opens_connections(&self, cx: &App) -> bool {
        self.terminals(cx).is_some()
            && self.workspace.upgrade().is_some_and(|workspace| {
                let workspace = workspace.read(cx);
                if self.source.context() == ContextId::General {
                    workspace.connection_directory().is_some()
                } else {
                    workspace.chat_connection_opener().is_some()
                }
            })
    }
    fn observe(&self, changed: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription> {
        let workspace = self.workspace.upgrade()?;
        Some(cx.subscribe(&workspace, move |_, _, cx| changed(cx)))
    }
    fn open_background(
        &self,
        profile: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId> {
        let owner = self.workspace.upgrade()?;
        if self.source.context() == ContextId::General {
            let directory = owner.read(cx).connection_directory()?;
            return directory.open_background(profile, &self.workspace, window, cx);
        }
        let opener = owner.read(cx).chat_connection_opener()?;
        match opener.open(profile, self.source, &self.workspace, window, cx) {
            Ok(item) => Some(item),
            Err(error) => {
                tracing::warn!(%error, "could not open chat's original workspace server");
                None
            }
        }
    }
    fn show_background(&self, item: EntityId, window: &mut Window, cx: &mut App) {
        let _ = self.workspace.update(cx, |workspace, cx| {
            if self.permits(workspace, item, cx) {
                let _ = workspace.show_context_background(self.source, item, window, cx);
            }
        });
    }
    fn close_background(&self, item: EntityId, window: &mut Window, cx: &mut App) {
        let _ = self.workspace.update(cx, |workspace, cx| {
            if self.permits(workspace, item, cx) {
                let _ = workspace.close_context_background(self.source, item, window, cx);
            }
        });
    }
    fn close_if_original(
        &self,
        item: EntityId,
        expected: Option<ExpectedSession>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.workspace
            .update(cx, |workspace, cx| {
                if !self.permits(workspace, item, cx) {
                    return false;
                }
                let entry = workspace
                    .context_terminals(self.source, cx)
                    .ok()
                    .and_then(|entries| {
                        entries
                            .into_iter()
                            .find(|entry| entry.item == item && entry.background)
                    });
                let Some(entry) = entry else {
                    return false;
                };
                let current = entry
                    .capabilities
                    .state()
                    .info(cx)
                    .map(|info| info.expected);
                if current != expected {
                    return false;
                }
                workspace
                    .close_context_background(self.source, item, window, cx)
                    .is_ok()
            })
            .unwrap_or(false)
    }
}
