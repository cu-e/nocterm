//! Where the connection lives: the personal space or one of the account's organizations.
use crate::{Installed, spaces::Spaces};
use gpui_kit::{
    AnyElement, App, SharedString, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    div,
    prelude::*,
};
use nocterm_connections_model::{profiles::ProfileId, replica::ReplicaScope};
use nocterm_ui::form;
use std::rc::Rc;

/// Said beside the save when the connection, in an organization, signs in with a personal
/// saved password or vault key: saving copies it into the organization's vault.
const SECRET_NOTE: &str = "Saving shares this connection's saved password or vault key with the organization's members. It leaves your personal vault unless a personal connection still uses it.";

pub(super) struct SpaceField {
    spaces: Spaces,
    selected: ReplicaScope,
    /// A move was requested for the save in progress.
    staged: bool,
    /// The request was recorded before the save, for a new connection.
    placed_early: bool,
    /// The save recorded its own request to share the connection's personal secrets with the
    /// organization it stays in.
    shared: bool,
}

impl SpaceField {
    pub(super) fn new(id: ProfileId, installed: &Installed, cx: &App) -> Self {
        let spaces = Spaces::of(installed, cx);
        let selected = spaces.scope(id, cx);
        Self {
            spaces,
            selected,
            staged: false,
            placed_early: false,
            shared: false,
        }
    }

    pub(super) fn select(&mut self, scope: ReplicaScope) {
        self.selected = scope;
    }

    /// Whether the chosen space is an organization's.
    fn organization_chosen(&self) -> bool {
        self.selected != ReplicaScope::Personal
    }

    /// The choice of space, offered only to an account that writes to an organization:
    /// a reader of organizations has nowhere else to put a connection. `on_select` receives
    /// the space a button stands for.
    pub(super) fn row(
        &self,
        on_select: impl Fn(ReplicaScope, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> Option<AnyElement> {
        let choices = self.spaces.choices();
        if choices.len() < 2 {
            return None;
        }
        let on_select = Rc::new(on_select);
        let buttons = choices.into_iter().map(|space| {
            let id = match &space.scope {
                ReplicaScope::Personal => "personal".to_owned(),
                ReplicaScope::Organization(org) => org.clone(),
            };
            let scope = space.scope.clone();
            let on_select = on_select.clone();
            Button::new(SharedString::from(format!("editor-space-{id}")))
                .small()
                .ghost()
                .label(space.name)
                .selected(self.selected == space.scope)
                .on_click(move |_, window, cx| on_select(scope.clone(), window, cx))
        });
        Some(form::stacked_row(
            "Space",
            "Personal connections stay with you; an organization's are shared with its members.",
            h_flex().gap_1().children(buttons),
            None,
            cx,
        ))
    }

    /// Said beside the save, on every page, when saving would hand the connection's personal
    /// password or vault key (`shares_secret`) to the organization chosen for it.
    pub(super) fn secret_note(&self, shares_secret: bool, cx: &App) -> Option<AnyElement> {
        (shares_secret && self.organization_chosen()).then(|| {
            div()
                .id("editor-space-secret-note")
                .test_support()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(SECRET_NOTE)
                .into_any_element()
        })
    }

    /// Records the chosen space, when it is not where the connection lives. Only a new
    /// connection is placed before its save, so its first synchronization already writes it
    /// there: nothing can move a connection that does not exist yet. An existing one moves
    /// once its save succeeded, so a synchronization running meanwhile cannot carry out the
    /// move of a save that is then refused. An organization's connection that stays where it
    /// is but is saved with a personal secret (`shares_secret`: the note of
    /// [`Self::secret_note`] was shown beside the save) asks for the secret to go with it,
    /// before the save that sends it.
    pub(super) fn stage(&mut self, id: ProfileId, is_new: bool, shares_secret: bool, cx: &mut App) {
        self.staged = self.selected != self.spaces.scope(id, cx);
        self.placed_early = self.staged && is_new;
        if self.placed_early {
            self.spaces.stage(id, self.selected.clone(), cx);
        } else if !self.staged && shares_secret && self.organization_chosen() {
            self.shared = self.spaces.share(id, self.selected.clone(), cx);
        }
    }

    /// The save is over: a successful one carries out the move, a failed one leaves no
    /// request behind.
    pub(super) fn settle(&mut self, id: ProfileId, saved: bool, cx: &mut App) {
        let placed_early = std::mem::take(&mut self.placed_early);
        if std::mem::take(&mut self.shared) && !saved {
            self.spaces.unstage(id, cx);
        }
        if !std::mem::take(&mut self.staged) {
            return;
        }
        if saved {
            self.spaces.place(id, self.selected.clone(), cx);
        } else if placed_early {
            self.spaces.unstage(id, cx);
        }
    }
}
