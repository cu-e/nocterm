//! The form that creates and edits a saved connection.

use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_connections_model::profiles::Profile;

use nocterm_ui::ActiveDesign as _;
use nocterm_workspace::Workspace;

use crate::{Installed, connect, spec_for_profile};

/// Two rows of icon tiles: 28 px tiles with a 4 px gap.
const ICON_ROWS_HEIGHT: f32 = 60.;

mod appearance;
mod profile;
mod render;
use profile::build_profile;

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

use appearance::AppearanceFields;
use authentication::AuthenticationFields;
use connection::ConnectionFields;
use launch::LaunchFields;
use nocterm_connections_model::draft::{AuthKind, ProfileDraft, ValidationField};
use state::EditState;
mod authentication;
mod connection;
mod launch;
mod state;

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

/// Opens the form in a dialog: empty for a new connection, filled in for
/// `profile`.
pub fn open_editor(
    installed: Installed,
    profile: Option<Profile>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = open_editor_view(installed, profile, workspace, window, cx);
}
fn open_editor_view(
    installed: Installed,
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
    let settings = installed.settings().clone();
    let editor = cx.new(|cx| ConnectionEditor::new(installed, profile, workspace, window, cx));
    let footer = cx.new(|cx| EditorFooter {
        editor: editor.clone(),
        _subscription: cx.observe(&editor, |_, _, cx| cx.notify()),
    });
    let dismissed = editor.read(cx).edit.dismissal();
    let focus = editor.read(cx).focus_handle(cx);
    let content = editor.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let editor = content.clone();
        let dismissed = dismissed.clone();
        dialog
            .title(title)
            .w(width)
            .bg(nocterm_ui::hsla(
                nocterm_ui::TerminalStyle::with_settings(
                    settings.setting::<nocterm_ui::TerminalSettings>(cx),
                    cx,
                )
                .background,
            ))
            .child(editor.clone())
            .footer(footer.clone())
            .on_close(move |_, _, _| dismissed.dismiss())
            .on_ok(move |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    if !editor
                        .connection
                        .description()
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
    deps: Installed,
    focus: FocusHandle,
    edit: EditState,
    workspace: WeakEntity<Workspace>,
    connection: ConnectionFields,
    authentication: AuthenticationFields,
    launch: LaunchFields,
    appearance: AppearanceFields,
    options: Entity<nocterm_ui::SessionOptionsEditor>,
    section: EditorSection,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

fn field(
    placeholder: &str,
    value: String,
    window: &mut Window,
    cx: &mut Context<ConnectionEditor>,
) -> Entity<InputState> {
    let placeholder = placeholder.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}
fn watch(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<ConnectionEditor>,
) -> Subscription {
    cx.subscribe_in(input, window, |this, _, event, _, cx| {
        if matches!(event, InputEvent::Change) && this.error.take().is_some() {
            cx.notify();
        }
    })
}
fn text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
impl ConnectionEditor {
    fn new(
        installed: Installed,
        profile: Option<Profile>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = installed.inputs().clone();
        let id = profile
            .as_ref()
            .map_or_else(|| inputs.next_id(), |profile| profile.id);
        let existing = profile.as_ref();
        let connection = ConnectionFields::new(existing, inputs.defaults(), window, cx);
        let authentication = AuthenticationFields::new(existing, window, cx);
        let launch = LaunchFields::new(existing, window, cx);
        let appearance = AppearanceFields::new(existing, &installed, window, cx);
        let options = cx.new(|cx| {
            nocterm_ui::SessionOptionsEditor::new(
                existing.map(|p| p.options.clone()).unwrap_or_default(),
                true,
                window,
                cx,
            )
        });
        let subscription = cx.observe(installed.facts(), |_, _, cx| cx.notify());
        Self {
            deps: installed,
            focus: cx.focus_handle(),
            edit: EditState::new(id, profile),
            workspace,
            connection,
            authentication,
            launch,
            appearance,
            options,
            section: EditorSection::Connection,
            error: None,
            _subscriptions: vec![subscription],
        }
    }
    fn fields(&self, cx: &App) -> ProfileDraft {
        let (metadata, endpoint) = self.connection.snapshot(cx);
        ProfileDraft {
            metadata,
            endpoint,
            authentication: self.authentication.snapshot(cx),
            options: Default::default(),
            launch: self.launch.snapshot(cx),
            appearance: self.appearance.snapshot(cx),
            jump_hosts: self
                .edit
                .original()
                .map(|profile| profile.jump_hosts.clone())
                .unwrap_or_default(),
        }
    }

    /// Saves the connection, and opens it when `and_connect` is set.
    fn save(&mut self, and_connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edit.try_begin() {
            return;
        }
        let profile = match self.validated_profile(cx) {
            Ok(profile) => profile,
            Err(error) => {
                if self.edit.finish() {
                    self.show_validation(error, window, cx);
                    cx.notify();
                }
                return;
            }
        };

        let saved = self.deps.connections().update(cx, |connections, cx| {
            connections.save_profile_checked(profile.clone(), self.edit.original().cloned(), cx)
        });
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.edit.finish() {
                    return;
                }
                match result {
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                    Ok(()) => {
                        this.edit.accept(profile.clone());
                        this.edit.dismiss();
                        window.close_dialog(cx);
                        if and_connect {
                            connect(
                                &this.deps,
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

    fn validated_profile(&self, cx: &App) -> Result<Profile, FormError> {
        let mut fields = self.fields(cx);
        fields.options = self
            .options
            .read(cx)
            .options(cx)
            .map_err(|error| FormError::new(ValidationField::Session, error.to_string()))?;
        build_profile(
            self.edit.id(),
            &fields,
            self.authentication.kind(),
            self.deps.inputs().defaults().user.as_deref(),
            self.deps.inputs().defaults().home.as_deref(),
        )
    }

    fn set_auth(&mut self, auth: AuthKind, window: &mut Window, cx: &mut Context<Self>) {
        self.authentication.select(auth);
        self.error = None;
        if auth == AuthKind::Key {
            self.authentication
                .key_path()
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
            ValidationField::Host => Some(self.connection.host()),
            ValidationField::Port => Some(self.connection.port()),
            ValidationField::User => Some(self.connection.user()),
            ValidationField::Key => Some(self.authentication.key_path()),
            ValidationField::Credential => Some(self.authentication.credential()),
            ValidationField::Arguments => Some(&self.launch.inputs()[1]),
            ValidationField::Environment => Some(&self.launch.inputs()[3]),
            ValidationField::Launch => Some(&self.launch.inputs()[0]),
            ValidationField::IconColor => Some(self.appearance.color()),
            ValidationField::Country => Some(self.appearance.country()),
            _ => None,
        };
        let focus = input
            .map(|input| input.read(cx).focus_handle(cx))
            .or_else(|| {
                matches!(error.field, ValidationField::Description)
                    .then(|| self.connection.description().read(cx).focus_handle(cx))
            });
        if let Some(focus) = focus {
            self.focus_after_render(section, focus, window, cx);
        }
        self.error = Some(error.message.into());
    }
}

impl Focusable for ConnectionEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let first = if self.edit.editing() {
            self.connection.name()
        } else {
            self.connection.host()
        };
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
                    .when(self.edit.original().is_some(), |form| form.child(
                        Button::new("editor-reload-current").small().ghost().label("Reload connection").disabled(self.edit.pending())
                            .tooltip("Discard this draft and load the current saved connection")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.edit.pending() || this.edit.is_dismissed() { return; }
                                let latest = this.deps.connections().read(cx).profiles(cx).get(this.edit.id()).cloned();
                                if let Some(profile) = latest {
                                    this.edit.dismiss();
                                    let workspace = this.workspace.clone();
                                    let installed = this.deps.clone();
                                    window.close_dialog(cx);
                                    // Finish dismissal before mounting the replacement and focusing its input.
                                    window.defer(cx, move |window, cx| open_editor(installed, Some(profile), workspace, window, cx));
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
                                this.edit.dismiss();
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("editor-save").label(if self.edit.pending() { "Saving…" } else { "Save" }).disabled(self.edit.pending()).on_click(
                            cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                        ),
                    )
                    .child(
                        Button::new("editor-save-connect")
                            .disabled(self.edit.pending())
                            .primary()
                            .label(if self.edit.editing() {
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
            EditorSection::Authentication => {
                self.authentication.credential().read(cx).focus_handle(cx)
            }
            EditorSection::Session => self.options.read(cx).focus_handle(cx),
            EditorSection::Launch if self.launch.overridden() => {
                self.launch.inputs()[0].read(cx).focus_handle(cx)
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
                editor.section == section && !editor.edit.is_dismissed()
            });
            if still_open {
                window.focus(&focus, cx);
            }
        });
    }
}
#[cfg(test)]
mod tests;
