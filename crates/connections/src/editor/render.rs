//! The editor's look: a page list beside titled sections of rows, like the
//! settings tab.
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Window,
    base::TestSupportExt as _,
    component::{
        Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState, Textarea},
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::{ActiveDesign as _, IconName, form};

use super::{AuthKind, ConnectionEditor, EditorSection};

/// A text field on the right of its label.
pub(super) fn field_row(
    label: &'static str,
    description: impl Into<SharedString>,
    input: &Entity<InputState>,
    cx: &App,
) -> AnyElement {
    form::row(
        label,
        description,
        div().w(rems(17.)).child(Input::new(input).small()),
        cx,
    )
}

impl EditorSection {
    fn icon(self) -> IconName {
        match self {
            Self::Connection => IconName::Server,
            Self::Authentication => IconName::KeyRound,
            Self::Session => IconName::SquareTerminal,
            Self::Launch => IconName::Zap,
        }
    }
}

impl ConnectionEditor {
    fn render_auth_choice(&self, cx: &mut Context<Self>) -> AnyElement {
        let choice = |id: &'static str,
                      label: &'static str,
                      kind: AuthKind,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .selected(self.auth == kind)
                .on_click(cx.listener(move |this, _, window, cx| this.set_auth(kind, window, cx)))
        };
        h_flex()
            .gap_1()
            .child(choice("auth-auto", "Automatic", AuthKind::Auto, cx))
            .child(choice("auth-password", "Password", AuthKind::Password, cx))
            .child(choice("auth-key", "Key file", AuthKind::Key, cx))
            .into_any_element()
    }

    fn connection_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
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
                        &self.launch_fields[2],
                        cx,
                    ),
                ],
                cx,
            ),
            form::section("Appearance", self.appearance_rows(cx), cx),
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

    fn authentication_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let hint = match self.auth {
            AuthKind::Auto => "Like ssh: the agent, then your default keys, then a password.",
            AuthKind::Password => "Always ask for the account's password.",
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
        };
        let choice = self.render_auth_choice(cx);
        let mut sign_in = vec![form::stacked_row("Method", hint, choice, None, cx)];
        if self.auth == AuthKind::Key {
            sign_in.push(field_row(
                "Private key",
                "Path to the key file.",
                &self.key_path,
                cx,
            ));
        }
        vec![
            form::section("Sign in", sign_in, cx),
            form::section(
                "Saved password",
                [field_row(
                    "Credential ID",
                    "Saved from the sign-in prompt. Empty: ask each time.",
                    &self.credential,
                    cx,
                )],
                cx,
            ),
        ]
    }

    fn launch_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![form::row(
            "Override launch settings",
            "Use this connection's own shell instead of the SSH defaults.",
            Switch::new("profile-launch-toggle")
                .checked(self.override_launch)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.override_launch = *checked;
                    cx.notify();
                })),
            cx,
        )];
        if self.override_launch {
            rows.extend([
                field_row(
                    "Remote executable",
                    "Empty: the account's shell.",
                    &self.launch_fields[0],
                    cx,
                ),
                field_row("Arguments", "A JSON array.", &self.launch_fields[1], cx),
                field_row(
                    "Initial directory",
                    "Where the program starts.",
                    &self.launch_fields[2],
                    cx,
                ),
                field_row(
                    "Environment",
                    "A JSON object. Keep secrets in the vault.",
                    &self.launch_fields[3],
                    cx,
                ),
                form::row(
                    "Shell integration",
                    "Track the working directory and commands.",
                    Switch::new("profile-integration-toggle")
                        .checked(self.launch_integration)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.launch_integration = *checked;
                            cx.notify();
                        })),
                    cx,
                ),
            ]);
        }
        vec![
            form::section("Launch", rows, cx),
            form::note("Launch options apply on the next start or reconnect.", cx),
        ]
    }
}

impl Render for ConnectionEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = &cx.design().layout;
        let height = rems(layout.connection_editor_height).to_pixels(window.rem_size());
        let available =
            (window.viewport_size().height - rems(10.).to_pixels(window.rem_size())).max(px(0.));
        let nav_width = rems(layout.connection_nav_width);
        let page = match self.section {
            EditorSection::Connection => self.connection_page(cx),
            EditorSection::Authentication => self.authentication_page(cx),
            EditorSection::Session => {
                vec![form::section(
                    "Session",
                    [div().py_3().child(self.options.clone()).into_any_element()],
                    cx,
                )]
            }
            EditorSection::Launch => self.launch_page(cx),
        };
        h_flex()
            .id("connection-editor")
            .track_focus(&self.focus)
            .items_start()
            .h(height.min(available))
            .min_h_0()
            .gap_4()
            .child(v_flex().w(nav_width).flex_shrink_0().gap(px(2.)).children(
                EditorSection::ALL.into_iter().map(|(section, id, label)| {
                    form::nav_item(id, section.icon(), label, self.section == section, cx)
                        .test_support()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_section(section, window, cx)
                        }))
                }),
            ))
            .child(
                v_flex()
                    .id("editor-section-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .pr_2()
                    .overflow_y_scroll()
                    .children(page),
            )
    }
}
