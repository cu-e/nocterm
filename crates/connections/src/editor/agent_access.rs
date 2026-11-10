//! Agent access of the saved connection being edited, offered in hub windows only.
//!
//! Turning it on writes the agent key's line through a session to the server: the one the user
//! has open, or else one opened without a tab with the connection's own saved credentials, the
//! way the sidebar opens it. A session that asks the user to sign in is shown in its tab.
mod session;

use crate::agent_access::{AgentAccessRefused, AgentServer, ServerAgentAccess};
use gpui_kit::{
    AnyElement, Context, SharedString, Subscription, Task, Window,
    base::TestSupportExt as _,
    component::{ActiveTheme as _, Disableable as _, switch::Switch},
    div,
    prelude::*,
};
use nocterm_connections_model::profiles::Profile;
use nocterm_ui::form;
use session::{BackgroundSession, POLL, Step};
use std::sync::Arc;

use super::ConnectionEditor;

const DESCRIPTION: &str = "Lets this hub's agents sign in with the account's agent key. Turning it on adds the key to the server's authorized_keys, connecting with the saved credentials when no session is open.";

/// What a change of agent access is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    /// A session to the server is being opened with its saved credentials.
    Connecting,
    /// The opened session waits for the user to sign in, in its tab.
    SigningIn,
    /// The server and the hub are being changed.
    Changing,
}

impl Progress {
    fn note(self) -> Option<&'static str> {
        match self {
            Self::Connecting => Some("Connecting with the saved credentials…"),
            Self::SigningIn => {
                Some("The server asks you to sign in: finish in its tab, then access is added.")
            }
            Self::Changing => None,
        }
    }
}

pub(super) struct AgentAccessField {
    port: Option<Arc<dyn ServerAgentAccess>>,
    /// What a change is doing; none while nothing changes. A change runs to its end even if the
    /// editor closes, so a written key line is always recorded on the hub.
    progress: Option<Progress>,
    problem: Option<SharedString>,
}

impl AgentAccessField {
    pub(super) fn new(port: Option<Arc<dyn ServerAgentAccess>>) -> Self {
        Self {
            port,
            progress: None,
            problem: None,
        }
    }

    /// Renders the editor again whenever the hub's agent servers change.
    pub(super) fn observe(&self, cx: &mut Context<ConnectionEditor>) -> Option<Subscription> {
        let editor = cx.weak_entity();
        let changed = move |cx: &mut gpui_kit::App| {
            let _ = editor.update(cx, |_, cx| cx.notify());
        };
        Some(self.port.as_ref()?.observe(Box::new(changed), cx))
    }

    /// The row for the saved connection `profile`; none outside hub windows or before the
    /// connection is saved.
    pub(super) fn row(
        &self,
        profile: Option<&Profile>,
        cx: &mut Context<ConnectionEditor>,
    ) -> Option<AnyElement> {
        let (port, profile) = (self.port.as_ref()?, profile?.clone());
        let granted = port.state(&AgentServer::of(&profile), cx).is_some();
        let switch = Switch::new("profile-agent-access")
            .checked(granted)
            .disabled(self.progress.is_some())
            .on_click(cx.listener(move |editor, checked: &bool, window, cx| {
                Self::change(editor, &profile, *checked, window, cx);
            }));
        let row = form::row("Agent access", DESCRIPTION, switch, cx);
        let note = match (&self.problem, self.progress.and_then(Progress::note)) {
            (Some(problem), _) => Some(("profile-agent-access-problem", problem.clone(), true)),
            (None, Some(note)) => Some(("profile-agent-access-progress", note.into(), false)),
            (None, None) => None,
        };
        Some(match note {
            Some((id, text, problem)) => {
                let text = if problem {
                    form::error_text(text, cx).into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(text)
                        .into_any_element()
                };
                div()
                    .id(id)
                    .test_support()
                    .child(row)
                    .child(text)
                    .into_any_element()
            }
            None => row,
        })
    }

    /// Grants or withdraws access for `profile`. Granting without an open session first opens
    /// one with the connection's saved credentials.
    fn change(
        editor: &mut ConnectionEditor,
        profile: &Profile,
        granted: bool,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) {
        let field = &mut editor.agent_access;
        let Some(port) = field.port.clone().filter(|_| field.progress.is_none()) else {
            return;
        };
        let server = AgentServer::of(profile);
        field.problem = None;
        if !granted || port.session_open(&server, cx) {
            field.progress = Some(Progress::Changing);
            let change = port.set(server, granted, cx);
            cx.spawn(async move |editor, cx| {
                let result = change.await;
                let _ = editor.update(cx, |editor, cx| editor.agent_access.finish(result, cx));
            })
            .detach();
        } else {
            match BackgroundSession::open(&editor.deps, &editor.workspace, profile, window, cx) {
                Some(session) => {
                    field.progress = Some(Progress::Connecting);
                    connect_then_grant(port, server, session, window, cx).detach();
                }
                None => field.problem = Some("The server could not be opened.".into()),
            }
        }
        cx.notify();
    }

    fn report(&mut self, progress: Progress, cx: &mut Context<ConnectionEditor>) {
        if self.progress != Some(progress) {
            self.progress = Some(progress);
            cx.notify();
        }
    }

    fn finish(
        &mut self,
        result: Result<(), AgentAccessRefused>,
        cx: &mut Context<ConnectionEditor>,
    ) {
        self.progress = None;
        self.problem = result.err().map(|refused| refused.reason().clone());
        cx.notify();
    }
}

/// Waits for `session` to come up, then grants access through it and closes it, unless the user
/// was shown it to sign in. The editor hears how it goes while it is open.
fn connect_then_grant(
    port: Arc<dyn ServerAgentAccess>,
    server: AgentServer,
    mut session: BackgroundSession,
    window: &mut Window,
    cx: &mut Context<ConnectionEditor>,
) -> Task<()> {
    cx.spawn_in(window, async move |editor, cx| {
        let connected = loop {
            cx.background_executor().timer(POLL).await;
            let step = cx.update(|window, cx| {
                let open = port.session_open(&server, cx);
                session.poll(open, window, cx)
            });
            match step {
                Ok(Step::Waiting { signing_in }) => {
                    let progress = if signing_in {
                        Progress::SigningIn
                    } else {
                        Progress::Connecting
                    };
                    let _ =
                        editor.update(cx, |editor, cx| editor.agent_access.report(progress, cx));
                }
                Ok(Step::Connected) => break Ok(()),
                Ok(Step::Failed(reason)) => break Err(AgentAccessRefused::new(reason)),
                Err(_) => return,
            }
        };
        let result = match connected {
            Ok(()) => {
                let _ = editor.update(cx, |editor, cx| {
                    editor.agent_access.report(Progress::Changing, cx);
                });
                match cx.update(|_, cx| port.set(server, true, cx)) {
                    Ok(change) => change.await,
                    Err(_) => return,
                }
            }
            Err(refused) => Err(refused),
        };
        let _ = cx.update(|window, cx| session.close(window, cx));
        let _ = editor.update(cx, |editor, cx| editor.agent_access.finish(result, cx));
    })
}
