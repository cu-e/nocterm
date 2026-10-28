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
    /// A move was requested for the save in progress.
    staged: bool,
}

impl SpaceField {
    pub(super) fn new(id: ProfileId, installed: &Installed, cx: &App) -> Self {
        let spaces = Spaces::of(installed, cx);
        let selected = spaces.scope(id, cx);
        Self {
            spaces,
            selected,
            staged: false,
        }
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

    /// Records the chosen space, when it is not where the connection lives, to be carried
    /// out by the save that follows.
    pub(super) fn stage(&mut self, id: ProfileId, cx: &mut App) {
        self.staged = self.selected != self.spaces.scope(id, cx);
        if self.staged {
            self.spaces.stage(id, self.selected.clone(), cx);
        }
    }

    /// The save is over: a successful one carries out the staged move, a failed one leaves
    /// no request behind.
    pub(super) fn settle(&mut self, id: ProfileId, saved: bool, cx: &mut App) {
        if !std::mem::take(&mut self.staged) {
            return;
        }
        if saved {
            self.spaces.sync(cx);
        } else {
            self.spaces.unstage(id, cx);
        }
    }
}
