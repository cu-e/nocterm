//! The form that creates and edits a saved connection.

use std::path::{Path, PathBuf};

use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_session::{Auth, DEFAULT_PORT, ShellLaunch, Target};
use nocterm_ui::ActiveDesign as _;
use nocterm_workspace::Workspace;

use crate::{
    Connections,
    model::{connect, spec_for_profile},
    store::{Profile, ProfileId},
};

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
}

/// Turns what was typed into a profile, or says what to fix.
fn build_profile(
    id: ProfileId,
    fields: &Fields,
    auth: AuthKind,
    local_user: Option<&str>,
    home: Option<&Path>,
) -> Result<Profile, FormError> {
    let host = fields.host.trim();
    if host.is_empty() {
        return Err(FormError::new(
            ValidationField::Host,
            "Enter a host name or address.",
        ));
    }
    if host.contains(char::is_whitespace) || host.contains('@') {
        return Err(FormError::new(
            ValidationField::Host,
            "The host is just the name or address, as in example.com.",
        ));
    }

    let port = match fields.port.trim() {
        "" => DEFAULT_PORT,
        port => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                FormError::new(
                    ValidationField::Port,
                    format!("`{port}` is not a port number."),
                )
            })?,
    };

    let user = match fields.user.trim() {
        "" => local_user.map(str::to_owned).ok_or_else(|| {
            FormError::new(ValidationField::User, "Enter the user to sign in as.")
        })?,
        user => user.to_owned(),
    };

    let auth = match auth {
        AuthKind::Auto => Auth::Auto,
        AuthKind::Password => Auth::Password,
        AuthKind::Key => match fields.key_path.trim() {
            "" => {
                return Err(FormError::new(
                    ValidationField::Key,
                    "Choose the private key file to sign in with.",
                ));
            }
            path => Auth::Key {
                path: expand_home(path, home),
            },
        },
    };

    let name = match fields.name.trim() {
        "" => host.to_owned(),
        name => name.to_owned(),
    };
    let group = Some(fields.group.trim())
        .filter(|group| !group.is_empty())
        .map(str::to_owned);

    let credential = match fields.credential.trim() {
        "" => None,
        id => Some(
            id.parse()
                .map_err(|message| FormError::new(ValidationField::Credential, message))?,
        ),
    };
    let launch = if fields.override_launch {
        let launch = ShellLaunch {
            program: Some(fields.program.trim().to_owned()).filter(|value| !value.is_empty()),
            args: if fields.args.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&fields.args).map_err(|_| {
                    FormError::new(
                        ValidationField::Arguments,
                        "Arguments must be a JSON array of strings.",
                    )
                })?
            },
            cwd: Some(fields.cwd.trim().to_owned()).filter(|value| !value.is_empty()),
            env: if fields.env.trim().is_empty() {
                Default::default()
            } else {
                serde_json::from_str(&fields.env).map_err(|_| {
                    FormError::new(
                        ValidationField::Environment,
                        "Environment must be a JSON object of string values.",
                    )
                })?
            },
            integration: fields.launch_integration,
        };
        launch
            .validate()
            .map_err(|message| FormError::new(ValidationField::Launch, message))?;
        Some(launch)
    } else {
        None
    };

    fields
        .options
        .validate()
        .map_err(|message| FormError::new(ValidationField::Session, message))?;
    if fields.description.len() > 16384 {
        return Err(FormError::new(
            ValidationField::Description,
            "Description must be at most 16384 bytes.",
        ));
    }
    Ok(Profile {
        description: fields.description.trim().to_owned(),
        options: fields.options.clone(),
        id,
        name,
        target: Target::new(user, host, port),
        auth,
        group,
        credential,
        launch,
    })
}

/// `~/x` relative to the home directory, as a shell would read it.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The user name to sign in as when none is given.
pub(crate) fn local_user() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|user| !user.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
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
    window.open_dialog(cx, move |dialog, _, _| {
        let editor = content.clone();
        let dismissed = dismissed.clone();
        dialog
            .title(title)
            .w(width)
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
    auth: AuthKind,
    section: EditorSection,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionEditor {
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
        let subscriptions = [
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

    fn render_field(label: &'static str, input: &Entity<InputState>, cx: &App) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(Input::new(input).small())
    }

    fn render_auth_choice(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(choice("auth-key", "Key File", AuthKind::Key, cx))
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
impl Render for ConnectionEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let layout = &cx.design().layout;
        let height = rems(layout.connection_editor_height).to_pixels(window.rem_size());
        let available =
            (window.viewport_size().height - rems(10.).to_pixels(window.rem_size())).max(px(0.));
        let nav_width = rems(layout.connection_nav_width);
        let description_height = rems(layout.connection_description_height);
        let auth_hint = match self.auth {
            AuthKind::Auto => "Like ssh: the agent, then your default keys, then a password.",
            AuthKind::Password => "Always ask for the account's password.",
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
        };
        let form = match self.section {
            EditorSection::Connection => v_flex().gap_3()
                .child(Self::render_field("Name", &self.name, cx))
                .child(h_flex().gap_2().items_start()
                    .child(div().flex_1().child(Self::render_field("Host", &self.host, cx)))
                    .child(div().w_24().child(Self::render_field("Port", &self.port, cx))))
                .child(Self::render_field("User", &self.user, cx))
                .child(Self::render_field("Folder", &self.group, cx))
                .child(div().text_xs().text_color(muted).child("Description"))
                .child(div().id("editor-description").test_support().h(description_height).child(Textarea::new(&self.description).h(description_height).aria_label("Connection description")))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Shared with AI agents when this connection is attached. Avoid secrets in descriptions.")),
            EditorSection::Authentication => v_flex().gap_3()
                .child(div().text_xs().text_color(muted).child("Sign in"))
                .child(self.render_auth_choice(cx))
                .child(div().text_xs().text_color(muted).child(auth_hint))
                .when(self.auth == AuthKind::Key, |form| form.child(Self::render_field("Private key", &self.key_path, cx)))
                .child(Self::render_field("Saved credential ID", &self.credential, cx))
                .child(div().text_xs().text_color(muted).child("Passwords are saved from the authentication prompt after successful sign in. Leave the ID empty to ask each time.")),
            EditorSection::Session => v_flex().gap_3().child(self.options.clone()),
            EditorSection::Launch => v_flex().gap_3()
                .child(Button::new("profile-launch-toggle").small().ghost().label("Override SSH launch settings").selected(self.override_launch).on_click(cx.listener(|this, _, _, cx| { this.override_launch = !this.override_launch; cx.notify(); })))
                .when(self.override_launch, |form| form
                    .child(Self::render_field("Remote executable (empty: default shell)", &self.launch_fields[0], cx))
                    .child(Self::render_field("Arguments as a JSON array", &self.launch_fields[1], cx))
                    .child(Self::render_field("Initial remote directory", &self.launch_fields[2], cx))
                    .child(Self::render_field("Environment as a JSON object", &self.launch_fields[3], cx))
                    .child(Button::new("profile-integration-toggle").small().ghost().label("Shell integration (directory and command tracking)").selected(self.launch_integration).on_click(cx.listener(|this, _, _, cx| { this.launch_integration = !this.launch_integration; cx.notify(); })))
                    .child(div().text_xs().text_color(muted).child("These options apply on the next start or reconnect. Keep secrets in the vault."))),
        };
        h_flex()
            .id("connection-editor")
            .track_focus(&self.focus)
            .items_start()
            .h(height.min(available))
            .min_h_0()
            .gap_4()
            .child(v_flex().w(nav_width).flex_shrink_0().gap_1().children(
                EditorSection::ALL.into_iter().map(|(section, id, label)| {
                    Button::new(id)
                        .w_full()
                        .ghost()
                        .label(label)
                        .selected(self.section == section)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_section(section, window, cx)
                        }))
                }),
            ))
            .child(
                div()
                    .id("editor-section-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, base::actions::Confirm, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn dialog_focus_accepts_typing_and_enter_opens_exactly_once(cx: &mut TestAppContext) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        cx.update_window(handle, |_, window, cx| {
            open_editor(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.input("example.test", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_none());
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                1
            );
            assert_eq!(
                opened.borrow().len(),
                1,
                "one Enter must open exactly one session"
            );
            assert_eq!(opened.borrow()[0].target.host, "example.test");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn dialog_confirm_keeps_invalid_form_open_then_saves_valid_form(cx: &mut TestAppContext) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        cx.update_window(handle, |_, window, cx| {
            open_editor(None, workspace.downgrade(), window, cx);
            window.render_frame(cx);
            window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_some());
            assert!(opened.borrow().is_empty());
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                0
            );
            window.input("valid.test", cx);
            window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_none());
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                1
            );
            assert_eq!(opened.borrow().len(), 1);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn sections_keep_drafts_and_multiline_description_enter_does_not_submit(
        cx: &mut TestAppContext,
    ) {
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let editor = cx
            .update_window(handle, |_, window, cx| {
                let editor = open_editor_view(None, workspace.downgrade(), window, cx);
                window.render_frame(cx);
                window.input("draft.test", cx);
                let focus = editor.read(cx).description.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
                window.render_frame(cx);
                window.input("first line", cx);
                window.press("enter", cx);
                window.input("second line", cx);
                editor
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("dialog").is_some(),
                "Textarea Enter must not submit"
            );
            assert!(opened.borrow().is_empty());
            assert_eq!(
                editor.read(cx).description.read(cx).value().as_ref(),
                "first line\nsecond line"
            );
            assert!(window.find("editor-description").bounds().size.height >= px(80.));
            window.click("editor-section-authentication", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("auth-password", cx);
            window.click("editor-section-session", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("session-term").is_some());
            window.click("editor-section-launch", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("profile-launch-toggle", cx);
            editor.update(cx, |editor, cx| {
                editor.launch_fields[0]
                    .update(cx, |input, cx| input.set_value("/bin/bash", window, cx));
            });
            window.click("editor-section-connection", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(editor.read(cx).fields(cx).host, "draft.test");
            assert_eq!(
                editor.read(cx).fields(cx).description,
                "first line\nsecond line"
            );
            assert_eq!(editor.read(cx).auth, AuthKind::Password);
            window.click("editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_none());
            let profile = Connections::global(cx)
                .read(cx)
                .profiles()
                .iter()
                .next()
                .unwrap();
            assert_eq!(profile.description, "first line\nsecond line");
            assert_eq!(profile.auth, Auth::Password);
            assert_eq!(
                profile.launch.as_ref().unwrap().program.as_deref(),
                Some("/bin/bash")
            );
            assert!(opened.borrow().is_empty(), "Save does not connect");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn hidden_invalid_fields_open_their_section_and_footer_stays_visible(cx: &mut TestAppContext) {
        let (handle, workspace, _) = crate::test_support::workspace(cx);
        cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(560.)));
        let editor = cx
            .update_window(handle, |_, window, cx| {
                let editor = open_editor_view(None, workspace.downgrade(), window, cx);
                window.render_frame(cx);
                window.input("valid.test", cx);
                editor.update(cx, |editor, cx| {
                    editor.auth = AuthKind::Key;
                    editor
                        .credential
                        .update(cx, |input, cx| input.set_value("not-an-id", window, cx));
                });
                window.click("editor-save", cx);
                editor
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(editor.read(cx).section == EditorSection::Authentication);
            assert!(
                editor
                    .read(cx)
                    .key_path
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            let save = window.find("editor-save").bounds();
            assert!(save.bottom() <= window.viewport_size().height);
            assert!(save.left() >= px(0.) && save.right() <= window.viewport_size().width);
            editor.update(cx, |editor, cx| {
                editor
                    .key_path
                    .update(cx, |input, cx| input.set_value("/tmp/key", window, cx));
            });
            window.click("editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                editor
                    .read(cx)
                    .credential
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert!(editor.read(cx).error.is_some());
            assert_eq!(
                Connections::global(cx).read(cx).profiles().iter().count(),
                0
            );
            editor.update(cx, |editor, cx| {
                editor
                    .credential
                    .update(cx, |input, cx| input.set_value("", window, cx));
                editor.options.update(cx, |options, cx| {
                    options.reset(
                        nocterm_session::SessionOptions {
                            term: Some("invalid TERM".into()),
                            ..Default::default()
                        },
                        window,
                        cx,
                    )
                });
                editor.select_section(EditorSection::Connection, window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("editor-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(editor.read(cx).section == EditorSection::Session);
            assert!(window.try_find("session-term").is_some());
            assert!(
                editor
                    .read(cx)
                    .options
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            );
            assert!(editor.read(cx).error.is_some());
            assert_eq!(editor.read(cx).fields(cx).host, "valid.test");
            assert!(window.find("editor-save").bounds().bottom() <= window.viewport_size().height);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn minimum_window_keeps_footer_in_view_for_every_section_and_wrapped_error(
        cx: &mut TestAppContext,
    ) {
        let (handle, workspace, _) = crate::test_support::workspace(cx);
        cx.simulate_window_resize(handle, gpui_kit::size(px(640.), px(400.)));
        let editor = cx.update_window(handle, |_, window, cx| {
            let editor = open_editor_view(None, workspace.downgrade(), window, cx);
            editor.update(cx, |editor, cx| {
                editor.error = Some("Connection changed while this editor was open. Review the current saved connection before applying this draft.".into());
                cx.notify();
            });
            editor
        }).unwrap();
        for (_, id, _) in EditorSection::ALL {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(id, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                for button in ["editor-save", "editor-save-connect", "editor-cancel"] {
                    let bounds = window.find(button).bounds();
                    assert!(
                        bounds.top() >= px(0.) && bounds.bottom() <= window.viewport_size().height,
                        "{button}: {bounds:?}"
                    );
                    assert!(
                        bounds.left() >= px(0.) && bounds.right() <= window.viewport_size().width
                    );
                }
            })
            .unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            window.click("editor-cancel", cx);
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_none());
            assert!(
                editor
                    .read(cx)
                    .dismissed
                    .load(std::sync::atomic::Ordering::Acquire)
            );
        })
        .unwrap();
    }

    fn reload_conflict_then_finish(cx: &mut TestAppContext, save: bool) {
        use futures::FutureExt as _;
        let (handle, workspace, opened) = crate::test_support::workspace(cx);
        let original = build_profile(
            ProfileId::generate(),
            &fields("old.test"),
            AuthKind::Auto,
            Some("tester"),
            None,
        )
        .unwrap();
        let mut latest = original.clone();
        latest.name = "Current saved connection".into();
        latest.target.host = "latest.test".into();
        latest.description = "Notes changed in another view".into();
        latest.auth = Auth::Password;
        let old_editor = cx
            .update_window(handle, |_, window, cx| {
                Connections::global(cx).update(cx, |connections, cx| {
                    connections
                        .save_profile(original.clone(), cx)
                        .now_or_never()
                        .unwrap()
                        .unwrap();
                });
                let editor =
                    open_editor_view(Some(original.clone()), workspace.downgrade(), window, cx);
                window.render_frame(cx);
                window.press("ctrl-a", cx);
                window.input("Unsaved stale draft", cx);
                Connections::global(cx).update(cx, |connections, cx| {
                    connections
                        .save_profile(latest.clone(), cx)
                        .now_or_never()
                        .unwrap()
                        .unwrap();
                });
                window.click("editor-save", cx);
                editor
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                old_editor.read(cx).error.is_some(),
                "stale snapshot must conflict"
            );
            assert_eq!(
                old_editor.read(cx).name.read(cx).value().as_ref(),
                "Unsaved stale draft"
            );
            window.click("editor-reload-current", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.focused_input(cx).unwrap().value(cx).as_ref(),
                latest.name
            );
            if save {
                window.press("ctrl-a", cx);
                window.input("Saved after reload", cx);
                window.click("editor-save", cx);
            } else {
                window.click("editor-cancel", cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("dialog").is_none(), "finishing the replacement must not reveal the stale editor");
            assert!(old_editor.read(cx).dismissed.load(std::sync::atomic::Ordering::Acquire));
            let connections = Connections::global(cx);
            let stored = connections.read(cx).profiles().get(original.id).unwrap();
            if save { latest.name = "Saved after reload".into(); }
            assert_eq!(stored, &latest, "reload must validate against the latest saved snapshot and preserve all of its fields");
            assert!(opened.borrow().is_empty());
        }).unwrap();
    }

    #[gpui_kit::test]
    fn conflict_reload_then_cancel_closes_the_old_editor(cx: &mut TestAppContext) {
        reload_conflict_then_finish(cx, false);
    }
    #[gpui_kit::test]
    fn conflict_reload_then_save_uses_latest_revision_and_leaves_no_old_editor(
        cx: &mut TestAppContext,
    ) {
        reload_conflict_then_finish(cx, true);
    }

    fn fields(host: &str) -> Fields {
        Fields {
            host: host.to_owned(),
            ..Fields::default()
        }
    }

    fn build(fields: &Fields, auth: AuthKind) -> Result<Profile, String> {
        build_profile(
            ProfileId::generate(),
            fields,
            auth,
            Some("me"),
            Some(Path::new("/home/me")),
        )
        .map_err(|error| error.message)
    }

    #[test]
    fn a_host_alone_is_enough() {
        let profile = build(&fields(" example.com "), AuthKind::Auto).unwrap();

        assert_eq!(profile.name, "example.com");
        assert_eq!(profile.target, Target::new("me", "example.com", 22));
        assert_eq!(profile.auth, Auth::Auto);
        assert_eq!(profile.group, None);
    }

    #[test]
    fn every_field_is_used() {
        let profile = build(
            &Fields {
                name: "Build".into(),
                host: "build.local".into(),
                port: "2222".into(),
                user: "ci".into(),
                group: " Work ".into(),
                key_path: "~/.ssh/ci".into(),
                ..Fields::default()
            },
            AuthKind::Key,
        )
        .unwrap();

        assert_eq!(profile.name, "Build");
        assert_eq!(profile.target, Target::new("ci", "build.local", 2222));
        assert_eq!(
            profile.auth,
            Auth::Key {
                path: PathBuf::from("/home/me/.ssh/ci")
            }
        );
        assert_eq!(profile.group.as_deref(), Some("Work"));
    }

    #[test]
    fn launch_override_and_credential_id_validate_and_preserve_arguments() {
        let mut draft = fields("example.com");
        assert!(build(&draft, AuthKind::Auto).unwrap().launch.is_none());
        let id = nocterm_session::CredentialId::generate().unwrap();
        draft.credential = id.to_string();
        draft.override_launch = true;
        draft.program = "/bin/bash".into();
        draft.args = r#"["-c","printf '%s' \"a b\""]"#.into();
        draft.env = r#"{"LANG":"C"}"#.into();
        draft.cwd = "/tmp/a b".into();
        draft.launch_integration = false;
        let profile = build(&draft, AuthKind::Auto).unwrap();
        assert_eq!(profile.credential, Some(id));
        let launch = profile.launch.unwrap();
        assert_eq!(launch.args, vec!["-c", "printf '%s' \"a b\""]);
        assert!(!launch.integration);
        draft.env = r#"{"INVALID;KEY":"value"}"#.into();
        assert!(build(&draft, AuthKind::Auto).is_err());
        draft.override_launch = false;
        draft.credential = "plain password".into();
        assert!(build(&draft, AuthKind::Auto).is_err());
    }

    #[test]
    fn mistakes_are_explained() {
        assert!(
            build(&fields(""), AuthKind::Auto)
                .unwrap_err()
                .contains("host")
        );
        assert!(build(&fields("me@host"), AuthKind::Auto).is_err());
        assert!(
            build(
                &Fields {
                    port: "ssh".into(),
                    ..fields("host")
                },
                AuthKind::Auto
            )
            .unwrap_err()
            .contains("port")
        );
        assert!(
            build(&fields("host"), AuthKind::Key)
                .unwrap_err()
                .contains("key")
        );
        assert!(
            build_profile(
                ProfileId::generate(),
                &fields("host"),
                AuthKind::Auto,
                None,
                None
            )
            .unwrap_err()
            .message
            .contains("user")
        );
    }
}
