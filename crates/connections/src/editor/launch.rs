//! Launch override controls and JSON text drafts.
use super::{ConnectionEditor, field, text, watch};
use gpui_kit::{App, Context, Entity, Subscription, Window, component::input::InputState};
use nocterm_connections_model::draft::LaunchDraft;
use nocterm_connections_model::profiles::Profile;
pub(super) struct LaunchFields {
    overridden: bool,
    integration: bool,
    inputs: [Entity<InputState>; 4],
    _subscriptions: Vec<Subscription>,
}
impl LaunchFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let launch = profile.and_then(|p| p.launch.clone()).unwrap_or_default();
        let overridden = profile.is_some_and(|p| p.launch.is_some());
        let integration = launch.integration;
        let inputs = [
            launch.program.unwrap_or_default(),
            serde_json::to_string(&launch.args).expect("argument serialization"),
            launch.cwd.unwrap_or_default(),
            serde_json::to_string(&launch.env).expect("environment serialization"),
        ]
        .map(|value| field("", value, window, cx));
        let subscriptions = inputs
            .iter()
            .map(|input| watch(input, window, cx))
            .collect();
        Self {
            overridden,
            integration,
            inputs,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn overridden(&self) -> bool {
        self.overridden
    }
    pub(super) fn set_override(&mut self, value: bool) {
        self.overridden = value;
    }
    pub(super) fn set_integration(&mut self, value: bool) {
        self.integration = value;
    }
    pub(super) fn inputs(&self) -> &[Entity<InputState>; 4] {
        &self.inputs
    }
    pub(super) fn snapshot(&self, cx: &App) -> LaunchDraft {
        LaunchDraft {
            override_launch: self.overridden,
            integration: self.integration,
            program: text(&self.inputs[0], cx),
            args: text(&self.inputs[1], cx),
            cwd: text(&self.inputs[2], cx),
            env: text(&self.inputs[3], cx),
        }
    }
}

use super::render::field_row;
use gpui_kit::{AnyElement, component::switch::Switch};
use nocterm_ui::form;

impl LaunchFields {
    pub(super) fn page(&self, cx: &mut Context<ConnectionEditor>) -> Vec<AnyElement> {
        let mut rows = vec![form::row(
            "Override launch settings",
            "Use this connection's own shell instead of the SSH defaults.",
            Switch::new("profile-launch-toggle")
                .checked(self.overridden)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.launch.set_override(*checked);
                    cx.notify();
                })),
            cx,
        )];
        if self.overridden {
            rows.extend([
                field_row(
                    "Remote executable",
                    "Empty: the account's shell.",
                    &self.inputs[0],
                    cx,
                ),
                field_row("Arguments", "A JSON array.", &self.inputs[1], cx),
                field_row(
                    "Initial directory",
                    "Where the program starts.",
                    &self.inputs[2],
                    cx,
                ),
                field_row(
                    "Environment",
                    "A JSON object. Keep secrets in the vault.",
                    &self.inputs[3],
                    cx,
                ),
                form::row(
                    "Shell integration",
                    "Track the working directory and commands.",
                    Switch::new("profile-integration-toggle")
                        .checked(self.integration)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.launch.set_integration(*checked);
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
