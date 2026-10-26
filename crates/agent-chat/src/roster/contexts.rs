//! Selected and retained chat collections keep immutable original dependencies.
use super::{ChatDeps, ChatList};
use gpui::EntityId;
use nocterm_ai::history::ContextId;
use std::collections::HashMap;

pub(super) struct ScopeChats {
    host: ChatDeps,
    chats: ChatList,
}
impl ScopeChats {
    fn new(host: ChatDeps) -> Self {
        Self {
            host,
            chats: ChatList::default(),
        }
    }
    pub(super) fn host(&self) -> &ChatDeps {
        &self.host
    }
    pub(super) fn chats(&self) -> &ChatList {
        &self.chats
    }
    pub(super) fn chats_mut(&mut self) -> &mut ChatList {
        &mut self.chats
    }
}
pub(super) struct ContextChats {
    selected: ScopeChats,
    hidden: HashMap<ContextId, ScopeChats>,
}
impl ContextChats {
    pub(super) fn new(host: ChatDeps) -> Self {
        Self {
            selected: ScopeChats::new(host),
            hidden: HashMap::new(),
        }
    }
    pub(super) fn current(&self) -> &ScopeChats {
        &self.selected
    }
    pub(super) fn current_mut(&mut self) -> &mut ScopeChats {
        &mut self.selected
    }
    pub(super) fn contains_any(&self, id: EntityId) -> bool {
        self.selected.chats.contains(id)
            || self.hidden.values().any(|scope| scope.chats.contains(id))
    }
    pub(super) fn release_quiet_hidden(&mut self, cx: &gpui::App) {
        self.hidden.retain(|_, scope| {
            let released: Vec<_> = scope
                .chats
                .iter()
                .filter(|thread| thread.read(cx).can_release_archive())
                .map(|thread| thread.entity_id())
                .collect();
            scope.chats.remove_admitted(&released);
            !scope.chats.is_empty()
        });
    }
    pub(super) fn origin(&self, id: EntityId) -> Option<&ScopeChats> {
        self.all().find(|scope| scope.chats.contains(id))
    }
    pub(super) fn scope_mut(&mut self, scope: ContextId) -> Option<&mut ScopeChats> {
        if self.selected.host.scope() == scope {
            Some(&mut self.selected)
        } else {
            self.hidden.get_mut(&scope)
        }
    }
    pub(super) fn all(&self) -> impl Iterator<Item = &ScopeChats> {
        std::iter::once(&self.selected).chain(self.hidden.values())
    }
    pub(super) fn remove_all(&mut self, ids: &[EntityId]) {
        self.selected.chats.remove_admitted(ids);
        self.hidden.retain(|_, scope| {
            scope.chats.remove_admitted(ids);
            !scope.chats.is_empty()
        });
    }
    pub(super) fn select(&mut self, host: ChatDeps) -> bool {
        let scope = host.scope();
        if scope == self.selected.host.scope() {
            self.selected.host = host;
            return false;
        }
        let mut next = self
            .hidden
            .remove(&scope)
            .unwrap_or_else(|| ScopeChats::new(host.clone()));
        // Future factories use the fresh original surface. Existing entities keep
        // their own previously captured ChatDeps/Seat and are never retargeted.
        next.host = host;
        let previous = std::mem::replace(&mut self.selected, next);
        if !previous.chats.is_empty() {
            self.hidden.insert(previous.host.scope(), previous);
        }
        true
    }
}
