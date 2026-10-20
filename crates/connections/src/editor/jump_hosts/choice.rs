//! How saved connections look where a jump host is chosen or listed: with the
//! server icon the sidebar shows for them.
use crate::{Installed, os::Os, panel::server_icon};
use gpui_kit::{
    AnyElement, App, SharedString, Window,
    component::{ActiveTheme as _, h_flex, select::SelectItem},
    div,
    prelude::*,
};
use nocterm_connections_model::profiles::{Profile, ProfileId};

/// A server's icon as the sidebar draws it: its chosen or detected system in
/// its colour, or the generic server icon.
#[derive(Clone, Default)]
pub(in crate::editor) struct ServerBadge {
    os: Option<&'static Os>,
    color: Option<SharedString>,
}

impl ServerBadge {
    pub(super) fn of(profile: &Profile, installed: &Installed, cx: &App) -> Self {
        Self {
            os: installed.shown_os(profile, cx),
            color: profile
                .icon_color
                .as_deref()
                .filter(|color| crate::os::is_valid_color(color))
                .map(|color| SharedString::from(color.to_owned())),
        }
    }

    pub(super) fn render(&self, cx: &App) -> AnyElement {
        server_icon(self.os, self.color.as_deref(), cx.theme().muted_foreground)
    }
}

/// A saved connection offered as a jump host.
#[derive(Clone)]
pub(in crate::editor) struct ConnectionChoice {
    id: ProfileId,
    title: SharedString,
    badge: ServerBadge,
}

impl ConnectionChoice {
    pub(super) fn new(
        title: SharedString,
        profile: &Profile,
        installed: &Installed,
        cx: &App,
    ) -> Self {
        Self {
            id: profile.id,
            title,
            badge: ServerBadge::of(profile, installed, cx),
        }
    }

    fn labelled(&self, cx: &App) -> AnyElement {
        h_flex()
            .gap_2()
            .min_w_0()
            .items_center()
            .child(self.badge.render(cx))
            .child(div().min_w_0().truncate().child(self.title.clone()))
            .into_any_element()
    }
}

impl SelectItem for ConnectionChoice {
    type Value = ProfileId;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.labelled(cx)
    }

    fn value(&self) -> &ProfileId {
        &self.id
    }
}
