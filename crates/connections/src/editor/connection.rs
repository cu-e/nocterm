//! Address, metadata and notes inputs.
use super::{ConnectionEditor, field, text, watch};
use crate::store::Profile;
use gpui_kit::{
    App, Context, Entity, Subscription, Window,
    component::input::{InputState, TextareaState},
};
use nocterm_connections_model::draft::{EndpointDraft, MetadataDraft};
use nocterm_session::DEFAULT_PORT;
pub(super) struct ConnectionFields {
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    group: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl ConnectionFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let name = field(
            "Shown in the sidebar; the host if empty",
            profile.map(|p| p.name.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let host = field(
            "example.com",
            profile.map(|p| p.target.host.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let port = field(
            "22",
            profile
                .map(|p| p.target.port)
                .filter(|port| *port != DEFAULT_PORT)
                .map(|port| port.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        let user = field(
            &super::local_user().unwrap_or_default(),
            profile.map(|p| p.target.user.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let group = field(
            "None",
            profile.and_then(|p| p.group.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(profile.map(|p| p.description.clone()).unwrap_or_default())
        });
        let subscriptions = [&name, &host, &port, &user, &group]
            .into_iter()
            .map(|input| watch(input, window, cx))
            .collect();
        Self {
            name,
            description,
            host,
            port,
            user,
            group,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn name(&self) -> &Entity<InputState> {
        &self.name
    }
    pub(super) fn host(&self) -> &Entity<InputState> {
        &self.host
    }
    pub(super) fn user(&self) -> &Entity<InputState> {
        &self.user
    }
    pub(super) fn port(&self) -> &Entity<InputState> {
        &self.port
    }
    pub(super) fn description(&self) -> &Entity<TextareaState> {
        &self.description
    }
    pub(super) fn snapshot(&self, cx: &App) -> (MetadataDraft, EndpointDraft) {
        (
            MetadataDraft {
                name: text(&self.name, cx),
                group: text(&self.group, cx),
                description: self.description.read(cx).value().to_string(),
            },
            EndpointDraft {
                host: text(&self.host, cx),
                port: text(&self.port, cx),
                user: text(&self.user, cx),
            },
        )
    }
}

use super::render::field_row;
use super::{appearance::AppearanceFields, launch::LaunchFields};
use crate::store::ProfileId;
use gpui_kit::{
    AnyElement,
    base::TestSupportExt as _,
    component::{
        Sizable as _, h_flex,
        input::{Input, Textarea},
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::{ActiveDesign as _, form};

impl ConnectionFields {
    pub(super) fn page(
        &self,
        launch: &LaunchFields,
        appearance: &AppearanceFields,
        id: ProfileId,
        editing: bool,
        cx: &mut Context<ConnectionEditor>,
    ) -> Vec<AnyElement> {
        let description_height = rems(cx.design().layout.connection_description_height);
        let address = h_flex()
            .w(rems(17.))
            .gap_1()
            .child(div().flex_1().child(Input::new(&self.host).small()))
            .child(div().w(rems(4.5)).child(Input::new(&self.port).small()));
        vec![
            form::section(
                "Server",
                [
                    field_row("Name", "Shown in the sidebar and on tabs.", &self.name, cx),
                    form::row("Address", "Host name or IP, and port.", address, cx),
                    field_row("User", "The account to sign in as.", &self.user, cx),
                ],
                cx,
            ),
            form::section(
                "Organisation",
                [
                    field_row(
                        "Folder",
                        "Groups the connection in the sidebar.",
                        &self.group,
                        cx,
                    ),
                    field_row(
                        "Starting directory",
                        "Where the shell starts; empty for home.",
                        &launch.inputs()[2],
                        cx,
                    ),
                ],
                cx,
            ),
            form::section("Appearance", appearance.rows(id, editing, cx), cx),
            form::section(
                "Notes",
                [form::stacked_row(
                    "Description",
                    "Shared with AI agents when this connection is attached. Avoid secrets.",
                    div()
                        .id("editor-description")
                        .test_support()
                        .h(description_height)
                        .child(
                            Textarea::new(&self.description)
                                .h(description_height)
                                .aria_label("Connection description"),
                        ),
                    None,
                    cx,
                )],
                cx,
            ),
        ]
    }
}
