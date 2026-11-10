//! The password a connection signs in with: typed here to keep it in the window's vault, or one
//! the vault already keeps, which may sign in to other servers as well.
//!
//! [`PasswordField`] owns the typed text, the chosen record, the vault's list of passwords and
//! the search that narrows it; the editor reads them when it saves and decides what to keep or
//! attach.
use crate::{SavedPassword, SavedPasswords};
use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Task, Window,
    component::input::{InputEvent, InputState},
};
use nocterm_session::{CredentialId, Secret};
use std::sync::Arc;

pub(super) struct PasswordField {
    passwords: Arc<dyn SavedPasswords>,
    /// A new password to keep; empty keeps the chosen one.
    input: Entity<InputState>,
    /// The saved password the connection signs in with, until another is chosen or it is
    /// forgotten.
    chosen: Option<CredentialId>,
    listed: Vec<SavedPassword>,
    /// What the list is narrowed to.
    query: Entity<InputState>,
    /// Bumped by every listing, so that a superseded answer is dropped.
    epoch: u64,
    listing: Option<Task<()>>,
    _query_changed: Subscription,
}

impl PasswordField {
    pub(super) fn new(
        passwords: Arc<dyn SavedPasswords>,
        chosen: Option<CredentialId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search saved passwords"));
        let query_changed = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            passwords,
            input,
            chosen,
            listed: Vec::new(),
            query,
            epoch: 0,
            listing: None,
            _query_changed: query_changed,
        }
    }

    pub(super) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    pub(super) fn chosen(&self) -> Option<CredentialId> {
        self.chosen
    }

    pub(super) fn listed(&self) -> &[SavedPassword] {
        &self.listed
    }

    pub(super) fn query(&self) -> &Entity<InputState> {
        &self.query
    }

    /// The listed passwords whose name has the searched text, with their place in the list.
    pub(super) fn matching(&self, cx: &gpui_kit::App) -> Vec<(usize, &SavedPassword)> {
        let query = self.query.read(cx).value().trim().to_lowercase();
        self.listed
            .iter()
            .enumerate()
            .filter(|(_, password)| {
                query.is_empty() || password.label.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// The saved password the connection signs in with, as listed.
    pub(super) fn chosen_listed(&self) -> Option<&SavedPassword> {
        let chosen = self.chosen?;
        self.listed.iter().find(|password| password.id == chosen)
    }

    /// The password typed to keep, if any.
    pub(super) fn typed(&self, cx: &gpui_kit::App) -> Option<Secret> {
        let typed = self.input.read(cx).value().to_string();
        (!typed.is_empty()).then(|| Secret::new(typed))
    }

    /// Signs in with the saved password at `index` of the list; a typed one is dropped.
    pub(super) fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(password) = self.listed.get(index) else {
            return;
        };
        self.chosen = Some(password.id);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Unlinks the saved password: the connection asks for it again.
    pub(super) fn forget(&mut self, cx: &mut Context<Self>) {
        self.chosen = None;
        cx.notify();
    }

    /// Reads the vault's passwords again; a vault that cannot list them offers none.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        let listed = self.passwords.list();
        self.listing = Some(cx.spawn(async move |this, cx| {
            let listed = listed.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.epoch == epoch {
                    this.listed = listed;
                    cx.notify();
                }
            });
        }));
    }
}
