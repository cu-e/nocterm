//! Agent access of the saved connection being edited, offered in hub windows only.
use crate::agent_access::{AgentServer, ServerAgentAccess};
use gpui_kit::{
    AnyElement, Context, SharedString,
    base::TestSupportExt as _,
    component::{Disableable as _, switch::Switch},
    div,
    prelude::*,
};
use nocterm_connections_model::profiles::Profile;
use nocterm_ui::form;
use std::sync::Arc;

use super::ConnectionEditor;

const DESCRIPTION: &str = "Lets this hub's agents sign in with the account's agent key. Turning it on adds the key to the server's authorized_keys through your open session.";

pub(super) struct AgentAccessField {
    port: Option<Arc<dyn ServerAgentAccess>>,
    /// A change is on its way. It runs to its end even if the editor closes, so a written key
    /// line is always recorded on the hub.
    changing: bool,
    problem: Option<SharedString>,
}

impl AgentAccessField {
    pub(super) fn new(port: Option<Arc<dyn ServerAgentAccess>>) -> Self {
        Self {
            port,
            changing: false,
            problem: None,
        }
    }

    /// The row for the saved connection `profile`; none outside hub windows or before the
    /// connection is saved.
    pub(super) fn row(
        &self,
        profile: Option<&Profile>,
        cx: &mut Context<ConnectionEditor>,
    ) -> Option<AnyElement> {
        let (port, server) = (self.port.as_ref()?, server_of(profile?));
        let granted = port.granted(&server, cx);
        let switch = Switch::new("profile-agent-access")
            .checked(granted)
            .disabled(self.changing)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                this.agent_access.set(server.clone(), *checked, cx);
            }));
        let row = form::row("Agent access", DESCRIPTION, switch, cx);
        Some(match &self.problem {
            Some(problem) => div()
                .id("profile-agent-access-problem")
                .test_support()
                .child(row)
                .child(form::error_text(problem.clone(), cx))
                .into_any_element(),
            None => row,
        })
    }

    fn set(&mut self, server: AgentServer, granted: bool, cx: &mut Context<ConnectionEditor>) {
        let Some(port) = self.port.clone().filter(|_| !self.changing) else {
            return;
        };
        self.problem = None;
        self.changing = true;
        let change = port.set(server, granted, cx);
        cx.spawn(async move |editor, cx| {
            let result = change.await;
            let _ = editor.update(cx, |editor, cx| {
                let field = &mut editor.agent_access;
                field.changing = false;
                field.problem = result.err().map(|refused| refused.reason().clone());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn server_of(profile: &Profile) -> AgentServer {
    AgentServer {
        connection: profile.id.to_string(),
        name: profile.name.clone(),
        host: profile.target.host.clone(),
        port: profile.target.port,
        user: profile.target.user.clone(),
    }
}
