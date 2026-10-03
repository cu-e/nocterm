//! The form that creates and edits a saved connection.

use std::path::{Path, PathBuf};

use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
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
) -> Result<Profile, String> {
    let host = fields.host.trim();
    if host.is_empty() {
        return Err("Enter a host name or address.".into());
    }
    if host.contains(char::is_whitespace) || host.contains('@') {
        return Err("The host is just the name or address, as in example.com.".into());
    }

    let port = match fields.port.trim() {
        "" => DEFAULT_PORT,
        port => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| format!("`{port}` is not a port number."))?,
    };

    let user = match fields.user.trim() {
        "" => local_user
            .map(str::to_owned)
            .ok_or_else(|| "Enter the user to sign in as.".to_owned())?,
        user => user.to_owned(),
    };

    let auth = match auth {
        AuthKind::Auto => Auth::Auto,
        AuthKind::Password => Auth::Password,
        AuthKind::Key => match fields.key_path.trim() {
            "" => return Err("Choose the private key file to sign in with.".into()),
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
        id => Some(id.parse().map_err(str::to_owned)?),
    };
    let launch = if fields.override_launch {
        let launch = ShellLaunch {
            program: Some(fields.program.trim().to_owned()).filter(|value| !value.is_empty()),
            args: if fields.args.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&fields.args)
                    .map_err(|_| "Arguments must be a JSON array of strings.".to_owned())?
            },
            cwd: Some(fields.cwd.trim().to_owned()).filter(|value| !value.is_empty()),
            env: if fields.env.trim().is_empty() {
                Default::default()
            } else {
                serde_json::from_str(&fields.env)
                    .map_err(|_| "Environment must be a JSON object of string values.".to_owned())?
            },
            integration: fields.launch_integration,
        };
        launch.validate()?;
        Some(launch)
    } else {
        None
    };

    fields.options.validate()?;
    if fields.description.len() > 16384 {
        return Err("Description must be at most 16384 bytes.".into());
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
    let title = if profile.is_some() {
        "Edit Connection"
    } else {
        "New Connection"
    };
    let width = rems(cx.design().layout.dialog_width).to_pixels(window.rem_size());
    let editor = cx.new(|cx| ConnectionEditor::new(profile, workspace, window, cx));
    let focus = editor.read(cx).focus_handle(cx);
    window.open_dialog(cx, move |dialog, _, _| {
        let editor = editor.clone();
        dialog
            .title(title)
            .w(width)
            .child(editor.clone())
            .on_ok(move |_, window, cx| {
                editor.update(cx, |editor, cx| editor.save(true, window, cx));
                // Saving closes the dialog only after validation succeeds.
                false
            })
    });
    window.focus(&focus, cx);
}

/// The connection form.
pub struct ConnectionEditor {
    id: ProfileId,
    editing: bool,
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
            id: existing.map_or_else(ProfileId::generate, |profile| profile.id),
            editing: existing.is_some(),
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
        let mut fields = self.fields(cx);
        fields.options = match self.options.read(cx).options(cx) {
            Ok(options) => options,
            Err(error) => {
                self.error = Some(error.into());
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
                self.error = Some(error.into());
                cx.notify();
                return;
            }
        };

        let saved = Connections::global(cx).update(cx, |connections, cx| {
            connections.save_profile(profile.clone(), cx)
        });
        if let Err(error) = saved {
            self.error = Some(error.into());
            cx.notify();
            return;
        }

        window.close_dialog(cx);
        if and_connect {
            connect(
                &self.workspace,
                spec_for_profile(&profile),
                Some(profile.id),
                window,
                cx,
            );
        }
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

impl Render for ConnectionEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let auth_hint = match self.auth {
            AuthKind::Auto => "Like ssh: the agent, then your default keys, then a password.",
            AuthKind::Password => "Always ask for the account's password.",
            AuthKind::Key => {
                "Sign in with one private key; its passphrase is asked for if it has one."
            }
        };

        v_flex()
            .gap_3()
            .child(Self::render_field("Name", &self.name, cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .child(Self::render_field("Host", &self.host, cx)),
                    )
                    .child(
                        div()
                            .w_24()
                            .child(Self::render_field("Port", &self.port, cx)),
                    ),
            )
            .child(Self::render_field("User", &self.user, cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child("Sign in"))
                    .child(self.render_auth_choice(cx))
                    .child(div().text_xs().text_color(muted).child(auth_hint)),
            )
            .when(self.auth == AuthKind::Key, |form| {
                form.child(Self::render_field("Private key", &self.key_path, cx))
            })
            .child(Self::render_field("Folder", &self.group, cx))
            .child(div().text_sm().child("Description"))
            .child(Textarea::new(&self.description))
            .child(self.options.clone())
            .child(Self::render_field("Saved credential ID", &self.credential, cx))
            .child(div().text_xs().text_color(muted).child("Passwords are saved from the authentication prompt after successful sign in. Leave the ID empty to ask each time."))
            .child(Button::new("profile-launch-toggle").small().ghost().label("Override SSH launch settings").selected(self.override_launch).on_click(cx.listener(|this, _, _, cx| { this.override_launch = !this.override_launch; cx.notify(); })))
            .when(self.override_launch, |form| form
                .child(Self::render_field("Remote executable (empty: default shell)", &self.launch_fields[0], cx))
                .child(Self::render_field("Arguments as a JSON array", &self.launch_fields[1], cx))
                .child(Self::render_field("Initial remote directory", &self.launch_fields[2], cx))
                .child(Self::render_field("Environment as a JSON object", &self.launch_fields[3], cx))
                .child(Button::new("profile-integration-toggle").small().ghost().label("Shell integration (directory and command tracking)").selected(self.launch_integration).on_click(cx.listener(|this, _, _, cx| { this.launch_integration = !this.launch_integration; cx.notify(); })))
                .child(div().text_xs().text_color(muted).child("These options apply on the next start or reconnect. Keep secrets in the vault.")))
            .when_some(self.error.clone(), |form, error| {
                form.child(div().text_sm().text_color(danger).child(error))
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
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("editor-save").label("Save").on_click(
                            cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                        ),
                    )
                    .child(
                        Button::new("editor-save-connect")
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
            .contains("user")
        );
    }
}
