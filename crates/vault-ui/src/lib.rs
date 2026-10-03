//! A singleton vault tab. Sensitive operations run on VaultService's worker.
use gpui_kit::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    SharedString, Subscription, Task, Window,
    component::{
        ActiveTheme as _, Disableable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::Secret;
use nocterm_ui::{ActiveDesign as _, ActiveSettings as _, IconName, SettingsStore};
use nocterm_vault::{CredentialInfo, VaultFuture, VaultService};
use nocterm_workspace::{Item, ItemEvent, OpenVault, Workspace};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Service(Arc<VaultService>);
impl Global for Service {}

/// Installs the shared service and returns it for authentication integration.
pub fn init(path: PathBuf, cx: &mut App) -> Result<Arc<VaultService>, nocterm_vault::VaultError> {
    let service = Arc::new(VaultService::new(
        path,
        Duration::from_secs(u64::from(cx.settings().vault.auto_lock_minutes) * 60),
    )?);
    cx.set_global(Service(service.clone()));
    cx.observe_global::<SettingsStore>(|cx| {
        cx.global::<Service>().0.set_auto_lock(Duration::from_secs(
            u64::from(cx.settings().vault.auto_lock_minutes) * 60,
        ));
    })
    .detach();
    Ok(service)
}

pub fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &OpenVault, window, cx| {
        if let Some(item) = workspace.find_item::<VaultView>() {
            workspace.activate_item_by_id(item.entity_id(), window, cx);
        } else {
            let item = cx.new(|cx| VaultView::new(window, cx));
            workspace.add_item(item, window, cx);
        }
    });
}

pub struct VaultView {
    service: Arc<VaultService>,
    password: Entity<InputState>,
    confirm: Entity<InputState>,
    focus: FocusHandle,
    busy: bool,
    message: Option<(SharedString, bool)>,
    records: Vec<CredentialInfo>,
    unlocked: bool,
    generation: u64,
    _watch: Task<()>,
    _subscriptions: Vec<Subscription>,
}
impl VaultView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let confirm = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Repeat master password")
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&password, window, |this, _, event, window, cx| {
                    if matches!(
                        event,
                        gpui_kit::component::input::InputEvent::PressEnter { .. }
                    ) && !this.service.is_unlocked()
                    {
                        this.submit(window, cx);
                    }
                }),
            ];
        let mut this = Self {
            service: cx.global::<Service>().0.clone(),
            focus: cx.focus_handle(),
            password,
            confirm,
            busy: false,
            message: None,
            records: Vec::new(),
            unlocked: false,
            generation: 0,
            _subscriptions: subscriptions,
            _watch: cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if !matches!(
                        cx.update(|window, app| this.update(app, |this, cx| {
                            let unlocked = this.service.is_unlocked();
                            if unlocked != this.unlocked {
                                this.unlocked = unlocked;
                                if !unlocked {
                                    this.records.clear();
                                    this.generation = this.generation.wrapping_add(1);
                                    this.busy = false;
                                    this.password
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    this.confirm
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                } else {
                                    this.refresh(cx);
                                }
                                cx.notify();
                            }
                        })),
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                }
            }),
        };
        if this.service.is_unlocked() {
            this.unlocked = true;
            this.refresh(cx);
        }
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let future = self.service.list();
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation || !this.service.is_unlocked() {
                    return;
                }
                match result {
                    Ok(records) => this.records = records,
                    Err(error) => this.message = Some((error.to_string().into(), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = Secret::new(self.password.read(cx).value().to_string());
        if (!self.service.exists() || self.service.is_unlocked())
            && password.expose() != self.confirm.read(cx).value().as_ref()
        {
            self.message = Some(("The master passwords do not match.".into(), true));
            cx.notify();
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
        let future = if self.service.is_unlocked() {
            self.service.change_password(password)
        } else if self.service.exists() {
            self.service.unlock(password)
        } else {
            self.service.create(password)
        };
        self.operation(future, cx);
    }
    fn operation(&mut self, future: VaultFuture<()>, cx: &mut Context<Self>) {
        self.busy = true;
        self.message = None;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.unlocked = this.service.is_unlocked();
                        this.message = Some(("Vault updated.".into(), false));
                        if this.unlocked {
                            this.refresh(cx);
                        }
                    }
                    Err(error) => this.message = Some((error.to_string().into(), true)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.service.lock();
        self.unlocked = false;
        self.records.clear();
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.message = Some(("Vault locked.".into(), false));
        cx.notify();
    }
}
impl EventEmitter<ItemEvent> for VaultView {}
impl Focusable for VaultView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.busy {
            self.focus.clone()
        } else {
            self.password.read(cx).focus_handle(cx)
        }
    }
}
impl Item for VaultView {
    fn tab_title(&self, _: &App) -> SharedString {
        "Credential vault".into()
    }
    fn tab_icon(&self, _: &App) -> IconName {
        IconName::KeyRound
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.confirm
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
}
impl Render for VaultView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unlocked = self.service.is_unlocked();
        let exists = self.service.exists();
        let width = rems(cx.design().layout.settings_width);
        let form = v_flex().p_6().gap_4().max_w(width)
                .child(div().text_lg().font_semibold().child("Credential vault"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(if unlocked { "Vault unlocked. Only explicitly saved passwords and private key passphrases appear here." } else if exists { "Unlock to use saved credentials. Metadata and secrets remain encrypted while locked." } else { "Create a portable vault encrypted with a master password. Forgetting this password prevents recovery. Choose a unique, strong password." }))
                .when(!unlocked, |form| form.child(Input::new(&self.password)).when(!exists, |form| form.child(Input::new(&self.confirm))))
                .when(unlocked, |form| form
                    .child(div().font_semibold().child("Saved credentials"))
                    .when(self.records.is_empty(), |form| form.child("No saved credentials. Choose Remember during SSH authentication after unlocking the vault."))
                    .children(self.records.iter().enumerate().map(|(index, record)| {
                        let id = record.id;
                        h_flex().gap_2().w_full().child(v_flex().flex_1().min_w_0().child(record.label.clone()).child(div().text_xs().text_color(cx.theme().muted_foreground).child(id.to_string())))
                            .child(Button::new(("vault-copy-id", index)).ghost().label("Copy ID").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()))))
                            .child(Button::new(("vault-delete", index)).ghost().label("Delete").disabled(self.busy).on_click(cx.listener(move |this, _, _, cx| this.operation(this.service.delete(id), cx))))
                    }))
                    .child(div().pt_4().font_semibold().child("Change master password"))
                    .child(Input::new(&self.password)).child(Input::new(&self.confirm)));
        let footer = v_flex()
            .flex_shrink_0()
            .px_6()
            .py_4()
            .gap_2()
            .max_w(width)
            .when_some(self.message.clone(), |footer, (message, error)| {
                footer.child(
                    div()
                        .text_sm()
                        .text_color(if error {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(message),
                )
            })
            .when(self.busy, |footer| footer.child("Working…"))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("vault-submit")
                            .primary()
                            .label(if unlocked {
                                "Change master password"
                            } else if exists {
                                "Unlock"
                            } else {
                                "Create vault"
                            })
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    )
                    .child(
                        Button::new("vault-lock")
                            .ghost()
                            .label("Lock")
                            .disabled(!unlocked && !self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
                    ),
            );
        v_flex()
            .size_full()
            .min_h_0()
            .track_focus(&self.focus)
            .child(
                div()
                    .id("vault-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form),
            )
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use gpui_kit::{TestAppContext, WindowOptions, test::TestWindowExt as _};
    use nocterm_settings::Settings;
    fn setup(
        cx: &mut TestAppContext,
        path: PathBuf,
    ) -> (
        gpui_kit::AnyWindowHandle,
        Entity<Workspace>,
        Arc<VaultService>,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(
                nocterm_ui::DesignTokens::builtin(),
                SettingsStore::in_memory(Settings::default()),
                cx,
            );
            let service = init(path, cx).unwrap();
            let (handle, workspace) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::new(window, cx);
                        register(&mut workspace);
                        workspace
                    })
                })
                .unwrap();
            (handle, workspace, service)
        })
    }
    #[gpui_kit::test]
    fn singleton_form_checks_confirmation_and_clears_fields_before_async_create(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (handle, workspace, service) = setup(cx, directory.path().join("vault"));
        cx.update_window(handle, |_, window, cx| {
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            window.render_frame(cx);
            window.dispatch_action(Box::new(OpenVault), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let view = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let view = workspace.read(cx).find_item::<VaultView>().unwrap();
                window.dispatch_action(Box::new(OpenVault), cx);
                assert_eq!(workspace.read(cx).items().count(), 1);
                view.update(cx, |view, cx| {
                    view.password.update(cx, |input, cx| {
                        input.set_value("long master password", window, cx)
                    });
                    view.confirm
                        .update(cx, |input, cx| input.set_value("different", window, cx));
                    view.submit(window, cx);
                    assert!(!view.busy);
                    assert!(view.message.as_ref().unwrap().1);
                    assert!(!service.exists());
                    view.confirm.update(cx, |input, cx| {
                        input.set_value("long master password", window, cx)
                    });
                    view.submit(window, cx);
                    assert!(view.busy);
                    assert!(view.password.read(cx).value().is_empty());
                    assert!(view.confirm.read(cx).value().is_empty());
                });
                view
            })
            .unwrap();
        block_on(service.list()).unwrap(); // Worker barrier, never used in production UI.
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                assert!(service.is_unlocked());
                assert!(!view.busy);
                assert!(!view.message.as_ref().unwrap().1);
                view.lock(window, cx);
                assert!(!service.is_unlocked());
                assert!(view.records.is_empty());
                view.password.update(cx, |input, cx| {
                    input.set_value("wrong password", window, cx)
                });
                view.submit(window, cx);
            })
        })
        .unwrap();
        let _ = block_on(service.list());
        cx.run_until_parked();
        assert!(cx.update(|cx| {
            view.read(cx)
                .message
                .as_ref()
                .is_some_and(|(_, error)| *error)
        }));
        assert!(!service.is_unlocked());
    }
}
