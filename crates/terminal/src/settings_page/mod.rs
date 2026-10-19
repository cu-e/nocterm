//! Terminal, local shell and SSH controls retain their installed settings.
mod controls;
mod render;
mod ssh;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Subscription,
    Task, Window, prelude::*,
};
use nocterm_settings::SettingsSection;
use nocterm_ui::settings_input::SettingsInputError as FieldError;
use nocterm_ui::{
    SessionOptionsEditor, SettingsPage, SettingsPageContext, SettingsPageFeedback,
    SettingsPageSpec, SettingsTextField, form,
};
use std::collections::BTreeMap;
trait PageState: Sized + 'static {
    fn settings(&self) -> &nocterm_store::SettingsHandle;
    fn fields(&self) -> &BTreeMap<SharedString, nocterm_ui::FieldPresentation>;
    fn fields_mut(&mut self) -> &mut BTreeMap<SharedString, nocterm_ui::FieldPresentation>;
    fn feedback(&self) -> &SettingsPageFeedback;
    fn field(&self, key: &str) -> &SettingsTextField {
        self.fields()[key].field()
    }
    fn save<S: SettingsSection>(
        &mut self,
        edit: impl FnOnce(&mut S) + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = self.settings().update::<S>(cx, edit);
        self.feedback().report(task, cx);
    }
    /// Adds a text setting of section `S` saved under `key`.
    fn add_field<S: SettingsSection>(
        &mut self,
        key: impl Into<SharedString>,
        read: impl Fn(&S) -> String + 'static,
        write: impl Fn(&mut S, &str) -> Result<(), FieldError> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        let field = SettingsTextField::new(
            self.settings().clone(),
            key.clone(),
            move |document| read(document.get::<S>()),
            move |document, text| {
                let mut section = document.get::<S>().clone();
                write(&mut section, text).map_err(nocterm_store::StoreError::edit)?;
                document.set(section);
                Ok(())
            },
            window,
            cx,
        );
        self.fields_mut()
            .insert(key, nocterm_ui::FieldPresentation::new(field, window, cx));
    }
}
struct TerminalSettingsPage {
    settings: nocterm_store::SettingsHandle,
    fields: BTreeMap<SharedString, nocterm_ui::FieldPresentation>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    feedback: SettingsPageFeedback,
}
impl PageState for TerminalSettingsPage {
    fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    fn fields(&self) -> &BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &self.fields
    }
    fn fields_mut(&mut self) -> &mut BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &mut self.fields
    }
    fn feedback(&self) -> &SettingsPageFeedback {
        &self.feedback
    }
}
impl TerminalSettingsPage {
    fn new(
        settings: nocterm_store::SettingsHandle,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let subscription = settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let mut this = Self {
            settings,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
            feedback: context.feedback().clone(),
        };
        render::terminal_fields(&mut this, window, cx);
        this
    }
}
impl Focusable for TerminalSettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl SettingsPage for TerminalSettingsPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values() {
            field.field().flush(window, cx);
        }
    }
}
impl Render for TerminalSettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::v_flex()
            .w_full()
            .child(form::page_header("Terminal", None, cx))
            .children(render::terminal(self, cx))
    }
}
struct LocalShellSettingsPage {
    settings: nocterm_store::SettingsHandle,
    fields: BTreeMap<SharedString, nocterm_ui::FieldPresentation>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    feedback: SettingsPageFeedback,
}
impl PageState for LocalShellSettingsPage {
    fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    fn fields(&self) -> &BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &self.fields
    }
    fn fields_mut(&mut self) -> &mut BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &mut self.fields
    }
    fn feedback(&self) -> &SettingsPageFeedback {
        &self.feedback
    }
}
impl LocalShellSettingsPage {
    fn new(
        settings: nocterm_store::SettingsHandle,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let subscription = settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let mut this = Self {
            settings,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
            feedback: context.feedback().clone(),
        };
        render::shell_fields::<nocterm_session::LocalShellSettings, _>(
            &mut this,
            "local",
            |s| &mut s.0,
            |s| &s.0,
            window,
            cx,
        );
        this
    }
}
impl Focusable for LocalShellSettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl SettingsPage for LocalShellSettingsPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values() {
            field.field().flush(window, cx);
        }
    }
}
impl Render for LocalShellSettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::v_flex()
            .w_full()
            .child(form::page_header("Local shell", None, cx))
            .children(render::local_shell(self, cx))
    }
}
struct SshSettingsPage {
    settings: nocterm_store::SettingsHandle,
    fields: BTreeMap<SharedString, nocterm_ui::FieldPresentation>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    feedback: SettingsPageFeedback,
    defaults: SessionDefaults,
}
impl PageState for SshSettingsPage {
    fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    fn fields(&self) -> &BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &self.fields
    }
    fn fields_mut(&mut self) -> &mut BTreeMap<SharedString, nocterm_ui::FieldPresentation> {
        &mut self.fields
    }
    fn feedback(&self) -> &SettingsPageFeedback {
        &self.feedback
    }
}
impl SshSettingsPage {
    fn new(
        settings: nocterm_store::SettingsHandle,
        context: SettingsPageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let original_window = window.window_handle();
        let subscription = settings.observe(cx, move |cx| {
            let _ = original_window.update(cx, |_, _, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let editor = ssh::new_session_options(&settings, window, cx);
        let options = cx.observe_in(&editor, window, |this, _, window, cx| {
            this.schedule_session_options(window, cx)
        });
        let mut this = Self {
            settings,
            fields: BTreeMap::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription, options],
            feedback: context.feedback().clone(),
            defaults: SessionDefaults {
                editor,
                error: None,
                save: None,
            },
        };
        ssh::add_ssh_fields(&mut this, window, cx);
        this
    }
}
impl Focusable for SshSettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl SettingsPage for SshSettingsPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in self.fields.values() {
            field.field().flush(window, cx);
        }
        if self.defaults.save.take().is_some() {
            self.save_session_options(cx);
        }
    }
}
impl Render for SshSettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::v_flex()
            .w_full()
            .child(form::page_header("SSH", None, cx))
            .children(ssh::ssh(self, cx))
    }
}
struct SessionDefaults {
    editor: Entity<SessionOptionsEditor>,
    error: Option<SharedString>,
    save: Option<Task<()>>,
}
impl SshSettingsPage {
    fn schedule_session_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.defaults.save = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(600))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.defaults.save = None;
                this.save_session_options(cx);
            });
        }));
    }
    fn save_session_options(&mut self, cx: &mut Context<Self>) {
        match self.defaults.editor.read(cx).options(cx) {
            Ok(options) => {
                self.defaults.error = None;
                let task = self.settings.edit(cx, move |document| {
                    ssh::apply_session_options(document, options)
                });
                self.feedback.report(task, cx);
            }
            Err(error) => {
                self.defaults.error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }
}
pub fn settings_pages(installed: &crate::Installed) -> [SettingsPageSpec; 3] {
    let settings = installed.settings().clone();
    [
        {
            let settings = settings.clone();
            SettingsPageSpec::contextual("terminal", "Terminal", move |context, window, cx| {
                cx.new(|cx| TerminalSettingsPage::new(settings.clone(), context, window, cx))
            })
            .with_icon(nocterm_ui::IconName::Terminal)
        },
        {
            let settings = settings.clone();
            SettingsPageSpec::contextual(
                "local-shell",
                "Local shell",
                move |context, window, cx| {
                    cx.new(|cx| LocalShellSettingsPage::new(settings.clone(), context, window, cx))
                },
            )
            .with_icon(nocterm_ui::IconName::SquareTerminal)
        },
        {
            let settings = settings.clone();
            SettingsPageSpec::contextual("ssh", "SSH", move |context, window, cx| {
                cx.new(|cx| SshSettingsPage::new(settings.clone(), context, window, cx))
            })
            .with_icon(nocterm_ui::IconName::Server)
        },
    ]
}
