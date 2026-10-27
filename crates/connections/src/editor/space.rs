//! Where the connection lives: the personal space or one of the account's organizations.
use super::ConnectionEditor;
use crate::{Installed, spaces::Spaces};
use gpui_kit::{
    AnyElement, App, Context, SharedString,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    prelude::*,
};
use nocterm_connections_model::{profiles::ProfileId, replica::ReplicaScope};
use nocterm_ui::form;

pub(super) struct SpaceField {
    spaces: Spaces,
    selected: ReplicaScope,
}

impl SpaceField {
    pub(super) fn new(id: ProfileId, installed: &Installed, cx: &App) -> Self {
        let spaces = Spaces::of(installed, cx);
        let selected = spaces.scope(id, cx);
        Self { spaces, selected }
    }

    pub(super) fn select(&mut self, scope: ReplicaScope) {
        self.selected = scope;
    }

    /// The choice of space, offered only to an account that writes to an organization:
    /// a reader of organizations has nowhere else to put a connection.
    pub(super) fn row(&self, cx: &mut Context<ConnectionEditor>) -> Option<AnyElement> {
        let choices = self.spaces.choices();
        if choices.len() < 2 {
            return None;
        }
        let buttons = choices.into_iter().map(|space| {
            let id = match &space.scope {
                ReplicaScope::Personal => "personal".to_owned(),
                ReplicaScope::Organization(org) => org.clone(),
            };
            let scope = space.scope.clone();
            Button::new(SharedString::from(format!("editor-space-{id}")))
                .small()
                .ghost()
                .label(space.name)
                .selected(self.selected == space.scope)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.space.select(scope.clone());
                    cx.notify();
                }))
        });
        Some(form::stacked_row(
            "Space",
            "Personal connections stay with you; an organization's are shared with its members.",
            h_flex().gap_1().children(buttons),
            None,
            cx,
        ))
    }

    /// Asks for the saved connection to move to the chosen space, when it is not there.
    pub(super) fn commit(&self, id: ProfileId, cx: &mut App) {
        if self.selected != self.spaces.scope(id, cx) {
            self.spaces.place(id, self.selected.clone(), cx);
        }
    }
}
