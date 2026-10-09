//! The form that creates and edits a saved connection.

use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{InputEvent, InputState, TextareaState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::{Auth, DEFAULT_PORT};
use nocterm_ui::ActiveDesign as _;
use nocterm_workspace::Workspace;

use crate::{
    Connections,
    model::{connect, spec_for_profile},
    store::{Profile, ProfileId},
};

/// Two rows of icon tiles: 28 px tiles with a 4 px gap.
const ICON_ROWS_HEIGHT: f32 = 60.;

mod appearance;
mod profile;
mod render;
pub(crate) use profile::local_user;
use profile::{build_profile, home_dir};

/// How the user signs in, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthKind {
    Auto,
    Password,
    Key,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorSection {
    Connection,
    Authentication,
    Session,
    Launch,
}
impl EditorSection {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Connection, "editor-section-connection", "Connection"),
        (
            Self::Authentication,
            "editor-section-authentication",
            "Authentication",
        ),
        (Self::Session, "editor-section-session", "Session"),
        (Self::Launch, "editor-section-launch", "Launch"),
    ];
}

#[derive(Clone, Copy, Debug)]
enum ValidationField {
    Host,
    Port,
    User,
    Key,
    Credential,
    Arguments,
    Environment,
    Launch,
    Session,
    Description,
    IconColor,
    Country,
}
#[derive(Debug)]
struct FormError {
    field: ValidationField,
    message: String,
}
impl FormError {
    fn new(field: ValidationField, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// The form's text, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fields {
    name: String,
    description: String,
    options: nocterm_session::SessionOptions,
    host: String,
    port: String,
    user: String,
    group: String,
    key_path: String,
    credential: String,
    override_launch: bool,
    launch_integration: bool,
    program: String,
    args: String,
    cwd: String,
    env: String,
    /// An operating system id; empty shows the detected system.
    icon: String,
    icon_color: String,
    country: String,
}

/// Opens the form in a dialog: empty for a new connection, filled in for
/// `profile`.
pub fn open_editor(
    profile: Option<Profile>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = open_editor_view(profile, workspace, window, cx);
}
fn open_editor_view(
    profile: Option<Profile>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConnectionEditor> {
    let title = if profile.is_some() {
        "Edit Connection"
    } else {
        "New Connection"
    };
    let width = rems(cx.design().layout.connection_editor_width).to_pixels(window.rem_size());
    let editor = cx.new(|cx| ConnectionEditor::new(profile, workspace, window, cx));
    let footer = cx.new(|cx| EditorFooter {
        editor: editor.clone(),
        _subscription: cx.observe(&editor, |_, _, cx| cx.notify()),
    });
    let dismissed = editor.read(cx).dismissed.clone();
    let focus = editor.read(cx).focus_handle(cx);
    let content = editor.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let editor = content.clone();
        let dismissed = dismissed.clone();
        dialog
            .title(title)
            .w(width)
            .bg(nocterm_ui::form::page_background(cx))
            .child(editor.clone())
            .footer(footer.clone())
            .on_close(move |_, _, _| dismissed.store(true, std::sync::atomic::Ordering::Release))
            .on_ok(move |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    if !editor
                        .description
                        .read(cx)
                        .focus_handle(cx)
                        .contains_focused(window, cx)
                    {
                        editor.save(true, window, cx);
                    }
                });
                // Saving closes the dialog only after validation succeeds.
                false
            })
    });
    window.focus(&focus, cx);
    editor
}

/// The connection form.
pub struct ConnectionEditor {
    focus: FocusHandle,
    id: ProfileId,
    editing: bool,
    original: Option<Profile>,
    pending: bool,
    dismissed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    workspace: WeakEntity<Workspace>,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    options: Entity<nocterm_ui::SessionOptionsEditor>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    group: Entity<InputState>,
    key_path: Entity<InputState>,
    credential: Entity<InputState>,
    override_launch: bool,
    launch_integration: bool,
    launch_fields: Vec<Entity<InputState>>,
    icon: Option<String>,
    icon_color: Entity<InputState>,
    /// Whether every icon is shown, not just the first two rows.
    icons_expanded: bool,
    country: Entity<InputState>,
    auth: AuthKind,
    section: EditorSection,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionEditor {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn new(
        profile: Option<Profile>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field =
            |placeholder: &str, value: String, window: &mut Window, cx: &mut Context<Self>| {
                let placeholder = placeholder.to_owned();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                })
            };
        let existing = profile.as_ref();
        let key_path = match existing.map(|profile| &profile.auth) {
            Some(Auth::Key { path }) => path.display().to_string(),
            _ => String::new(),
        };
        let auth = match existing.map(|profile| &profile.auth) {
            Some(Auth::Password) => AuthKind::Password,
            Some(Auth::Key { .. }) => AuthKind::Key,
            _ => AuthKind::Auto,
        };
        let local_user = local_user().unwrap_or_default();

        let name = field(
            "Shown in the sidebar; the host if empty",
            existing.map(|p| p.name.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let host = field(
            "example.com",
            existing.map(|p| p.target.host.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let port = field(
            "22",
            existing
                .map(|p| p.target.port)
                .filter(|port| *port != DEFAULT_PORT)
                .map(|port| port.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        let user = field(
            &local_user,
            existing.map(|p| p.target.user.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let group = field(
            "None",
            existing.and_then(|p| p.group.clone()).unwrap_or_default(),
            window,
            cx,
        );
        let key_path = field("~/.ssh/id_ed25519", key_path, window, cx);

        let credential = field(
            "Credential ID copied from Vault (optional)",
            existing
                .and_then(|profile| profile.credential)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        let launch = existing
            .and_then(|profile| profile.launch.clone())
            .unwrap_or_default();
        let override_launch = existing.is_some_and(|profile| profile.launch.is_some());
        let launch_integration = launch.integration;
        let launch_fields: Vec<_> = [
            launch.program.unwrap_or_default(),
            serde_json::to_string(&launch.args).expect("argument serialization"),
            launch.cwd.unwrap_or_default(),
            serde_json::to_string(&launch.env).expect("environment serialization"),
        ]
        .into_iter()
        .map(|value| field("", value, window, cx))
        .collect();
        let icon_color = field(
            "Brand colour",
            existing
                .and_then(|profile| profile.icon_color.clone())
                .unwrap_or_default(),
            window,
            cx,
        );
        let country = field(
            "Auto",
            existing
                .and_then(|profile| profile.country.clone())
                .map(|code| code.to_uppercase())
                .unwrap_or_default(),
            window,
            cx,
        );
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(existing.map(|p| p.description.clone()).unwrap_or_default())
        });
        let options = cx.new(|cx| {
            nocterm_ui::SessionOptionsEditor::new(
                existing.map(|p| p.options.clone()).unwrap_or_default(),
                true,
                window,
                cx,
            )
        });
        let mut subscriptions: Vec<_> = [
            &name,
            &host,
            &port,
            &user,
            &group,
            &key_path,
            &credential,
            &launch_fields[0],
            &launch_fields[1],
            &launch_fields[2],
            &launch_fields[3],
        ]
        .into_iter()
        .map(|input| {
            cx.subscribe_in(input, window, |this, _, event, _, cx| match event {
                InputEvent::Change if this.error.take().is_some() => cx.notify(),
                _ => {}
            })
        })
        .collect();
        // The preview follows the colour as it is typed.
        subscriptions.push(
            cx.subscribe_in(&icon_color, window, |this, _, event, _, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.observe(&crate::ServerFacts::global(cx), |_, _, cx| cx.notify()));
        // The flag preview loads as a code is typed.
        subscriptions.push(
            cx.subscribe_in(&country, window, |this, input, event, _, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    let code = input.read(cx).value().trim().to_ascii_lowercase();
                    crate::ServerFacts::global(cx)
                        .update(cx, |facts, cx| facts.ensure_flag(&code, cx));
                    cx.notify();
                }
            }),
        );

        Self {
            focus: cx.focus_handle(),
            id: existing.map_or_else(ProfileId::generate, |profile| profile.id),
            editing: existing.is_some(),
            original: profile.clone(),
            pending: false,
            dismissed: Default::default(),
            workspace,
            name,
            description,
            options,
            host,
            port,
            user,
            group,
            key_path,
            credential,
            override_launch,
            launch_integration,
            launch_fields,
            icon: existing.and_then(|profile| profile.icon.clone()),
            icon_color,
            icons_expanded: false,
            country,
            auth,
            section: EditorSection::Connection,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn fields(&self, cx: &App) -> Fields {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        Fields {
            name: text(&self.name),
            description: self.description.read(cx).value().to_string(),
            options: Default::default(),
            host: text(&self.host),
            port: text(&self.port),
            user: text(&self.user),
            group: text(&self.group),
            key_path: text(&self.key_path),
            credential: text(&self.credential),
            override_launch: self.override_launch,
            launch_integration: self.launch_integration,
            program: text(&self.launch_fields[0]),
            args: text(&self.launch_fields[1]),
            cwd: text(&self.launch_fields[2]),
            env: text(&self.launch_fields[3]),
            icon: self.icon.clone().unwrap_or_default(),
            icon_color: text(&self.icon_color),
            country: text(&self.country),
        }
    }

    /// Saves the connection, and opens it when `and_connect` is set.
    fn save(&mut self, and_connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.dismissed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let mut fields = self.fields(cx);
        fields.options = match self.options.read(cx).options(cx) {
            Ok(options) => options,
            Err(error) => {
                self.show_validation(FormError::new(ValidationField::Session, error), window, cx);
                cx.notify();
                return;
            }
        };
        let profile = match build_profile(
            self.id,
            &fields,
            self.auth,
            local_user().as_deref(),
            home_dir().as_deref(),
        ) {
            Ok(profile) => profile,
            Err(error) => {
                self.show_validation(error, window, cx);
                cx.notify();
                return;
            }
        };

        let saved = Connections::global(cx).update(cx, |connections, cx| {
            connections.save_profile_checked(profile.clone(), self.original.clone(), cx)
        });
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.dismissed.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                match result {
                    Err(error) => {
                        this.error = Some(error.into());
                        cx.notify();
                    }
                    Ok(()) => {
                        this.original = Some(profile.clone());
                        this.dismissed
                            .store(true, std::sync::atomic::Ordering::Release);
                        window.close_dialog(cx);
                        if and_connect {
                            connect(
                                &this.workspace,
                                spec_for_profile(&profile),
                                Some(profile.id),
                                window,
                                cx,
                            );
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn set_auth(&mut self, auth: AuthKind, window: &mut Window, cx: &mut Context<Self>) {
        self.auth = auth;
        self.error = None;
        if auth == AuthKind::Key {
            self.key_path
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn show_validation(&mut self, error: FormError, window: &mut Window, cx: &mut Context<Self>) {
        let section = match error.field {
            ValidationField::Key | ValidationField::Credential => EditorSection::Authentication,
            ValidationField::Arguments | ValidationField::Environment | ValidationField::Launch => {
                EditorSection::Launch
            }
            ValidationField::Session => EditorSection::Session,
            _ => EditorSection::Connection,
        };
        self.select_section(section, window, cx);
        let input = match error.field {
            ValidationField::Host => Some(&self.host),
            ValidationField::Port => Some(&self.port),
            ValidationField::User => Some(&self.user),
            ValidationField::Key => Some(&self.key_path),
            ValidationField::Credential => Some(&self.credential),
            ValidationField::Arguments => Some(&self.launch_fields[1]),
            ValidationField::Environment => Some(&self.launch_fields[3]),
            ValidationField::Launch => Some(&self.launch_fields[0]),
            ValidationField::IconColor => Some(&self.icon_color),
            ValidationField::Country => Some(&self.country),
            _ => None,
        };
        let focus = input
            .map(|input| input.read(cx).focus_handle(cx))
            .or_else(|| {
                matches!(error.field, ValidationField::Description)
                    .then(|| self.description.read(cx).focus_handle(cx))
            });
        if let Some(focus) = focus {
            self.focus_after_render(section, focus, window, cx);
        }
        self.error = Some(error.message.into());
    }
}

impl Focusable for ConnectionEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let first = if self.editing { &self.name } else { &self.host };
        first.read(cx).focus_handle(cx)
    }
}

struct EditorFooter {
    editor: Entity<ConnectionEditor>,
    _subscription: Subscription,
}
impl Render for EditorFooter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.editor
            .update(cx, |editor, cx| editor.render_footer(cx))
    }
}
impl ConnectionEditor {
    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let danger = cx.theme().danger;
        v_flex().id("editor-footer").gap_2()
            .when_some(self.error.clone(), |form, error| {
                form.child(div().text_sm().text_color(danger).child(error))
                    .when(self.original.is_some(), |form| form.child(
                        Button::new("editor-reload-current").small().ghost().label("Reload connection").disabled(self.pending)
                            .tooltip("Discard this draft and load the current saved connection")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.pending || this.dismissed.load(std::sync::atomic::Ordering::Acquire) { return; }
                                let latest = Connections::global(cx).read(cx).profiles().get(this.id).cloned();
                                if let Some(profile) = latest {
                                    this.dismissed.store(true, std::sync::atomic::Ordering::Release);
                                    let workspace = this.workspace.clone();
                                    window.close_dialog(cx);
                                    // Finish dismissal before mounting the replacement and focusing its input.
                                    window.defer(cx, move |window, cx| open_editor(Some(profile), workspace, window, cx));
                                }
                                else { this.error = Some("This connection was deleted. Close the editor to create a new connection.".into()); cx.notify(); }
                            }))
                    ))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new("editor-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dismissed.store(true, std::sync::atomic::Ordering::Release);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("editor-save").label(if self.pending { "Saving…" } else { "Save" }).disabled(self.pending).on_click(
                            cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                        ),
                    )
                    .child(
                        Button::new("editor-save-connect")
                            .disabled(self.pending)
                            .primary()
                            .label(if self.editing {
                                "Save and Connect"
                            } else {
                                "Connect"
                            })
                            .font_semibold()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save(true, window, cx)),
                            ),
                    ),
            )
    }
    fn select_section(
        &mut self,
        section: EditorSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.section = section;
        let focus = match section {
            EditorSection::Connection => self.focus_handle(cx),
            EditorSection::Authentication => self.credential.read(cx).focus_handle(cx),
            EditorSection::Session => self.options.read(cx).focus_handle(cx),
            EditorSection::Launch if self.override_launch => {
                self.launch_fields[0].read(cx).focus_handle(cx)
            }
            EditorSection::Launch => self.focus.clone(),
        };
        // Mount the new section before moving focus into its native field.
        self.focus_after_render(section, focus, window, cx);
        cx.notify();
    }

    fn focus_after_render(
        &self,
        section: EditorSection,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let still_open = editor.upgrade().is_some_and(|editor| {
                let editor = editor.read(cx);
                editor.section == section
                    && !editor.dismissed.load(std::sync::atomic::Ordering::Acquire)
            });
            if still_open {
                window.focus(&focus, cx);
            }
        });
    }
}
#[cfg(test)]
mod tests;
