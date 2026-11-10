//! One part of the User section, drawn where the Plugins page puts it.
//!
//! The section draws the part inside its own update, so the part's controls act on the section;
//! the part view redraws whenever the section changes.

use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, Window};

use super::{
    section::UserPluginsSection,
    section_view::{self, UserPart},
};

/// A part of the User section in the Plugins page.
pub struct UserPartView {
    section: Entity<UserPluginsSection>,
    part: UserPart,
    _changed: Subscription,
}

impl UserPartView {
    pub fn new(
        section: Entity<UserPluginsSection>,
        part: UserPart,
        cx: &mut Context<Self>,
    ) -> Self {
        let changed = cx.observe(&section, |_, _, cx| cx.notify());
        Self {
            section,
            part,
            _changed: changed,
        }
    }
}

impl Render for UserPartView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.section
            .update(cx, |section, cx| section_view::part(section, part, cx))
    }
}
