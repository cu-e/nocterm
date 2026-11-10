//! The Credentials tab of the Vault page: what the vault keeps, by name, searched by name or
//! server. A password can be added under a name of its own (in a dialog) and then sign in to
//! several servers, which are listed under it; any credential can be renamed or deleted, and a
//! password detached from a server.
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, SharedString, Subscription,
    Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_session::CredentialId;
use nocterm_ui::{IconName, form};
use nocterm_vault::{CredentialBinding, CredentialInfo, VaultAccess, VaultFuture};
use std::sync::Arc;

pub(crate) struct CredentialsTab {
    service: Arc<dyn VaultAccess>,
    records: Vec<CredentialInfo>,
    /// Bumped when the vault locks or the page closes, so that a late answer is dropped.
    generation: u64,
    busy: bool,
    message: Option<(SharedString, bool)>,
    /// What the list is narrowed to: a name or a server.
    query: Entity<InputState>,
    renaming: Option<Renaming>,
    _subscriptions: Vec<Subscription>,
}

/// The record being renamed, and its name field.
struct Renaming {
    id: CredentialId,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl CredentialsTab {
    pub(crate) fn new(
        service: Arc<dyn VaultAccess>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search by name or server"));
        let subscriptions = vec![cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        Self {
            service,
            records: Vec::new(),
            generation: 0,
            busy: false,
            message: None,
            query,
            renaming: None,
            _subscriptions: subscriptions,
        }
    }

    /// Reads the records again.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
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

    /// Forgets what the vault showed and every draft: it locked or the page closed.
    pub(crate) fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.records.clear();
        self.busy = false;
        self.message = None;
        self.renaming = None;
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// The records whose name or server has the searched text.
    fn matching(&self, cx: &gpui_kit::App) -> Vec<CredentialInfo> {
        let query = self.query.read(cx).value().trim().to_lowercase();
        self.records
            .iter()
            .filter(|record| {
                query.is_empty()
                    || record.label.to_lowercase().contains(&query)
                    || record
                        .binding
                        .targets()
                        .iter()
                        .any(|target| target.to_string().to_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    fn start_rename(&mut self, id: CredentialId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.records.iter().find(|record| record.id == id) else {
            return;
        };
        let label = record.label.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(label));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.finish_rename(cx),
            _ => {}
        });
        self.renaming = Some(Renaming {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(Renaming { id, input, .. }) = self.renaming.take() else {
            return;
        };
        let label = input.read(cx).value().trim().to_owned();
        let Some(record) = self.records.iter().find(|record| record.id == id) else {
            return;
        };
        if label.is_empty() || label == record.label {
            cx.notify();
            return;
        }
        let renamed = self.service.update(id, label, record.binding.clone());
        self.operation(renamed, cx);
    }

    /// Stops the password `id` signing in to its server at `index`.
    fn detach(&mut self, id: CredentialId, index: usize, cx: &mut Context<Self>) {
        let Some(record) = self.records.iter().find(|record| record.id == id) else {
            return;
        };
        let mut targets = record.binding.targets().to_vec();
        if index >= targets.len() {
            return;
        }
        targets.remove(index);
        let detached = self.service.update(
            id,
            record.label.clone(),
            CredentialBinding::password(targets),
        );
        self.operation(detached, cx);
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
                if let Err(error) = result {
                    this.message = Some((error.to_string().into(), true));
                }
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl CredentialsTab {
    /// One credential: its name (or the field renaming it), what it is and where it signs in,
    /// and its buttons.
    fn row(&self, index: usize, record: &CredentialInfo, cx: &mut Context<Self>) -> AnyElement {
        let id = record.id;
        let theme = cx.theme().clone();
        let name = match &self.renaming {
            Some(renaming) if renaming.id == id => div()
                .w(rems(16.))
                .child(
                    Input::new(&renaming.input)
                        .id(("vault-rename-field", index))
                        .small(),
                )
                .into_any_element(),
            _ => div()
                .text_sm()
                .child(record.label.clone())
                .into_any_element(),
        };
        let servers = record.binding.targets();
        let detach = servers.iter().enumerate().map(|(at, target)| {
            Button::new(SharedString::from(format!("vault-detach-{index}-{at}")))
                .xsmall()
                .ghost()
                .label(target.to_string())
                .icon(IconName::X)
                .tooltip("Stop signing in to this server with it")
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| this.detach(id, at, cx)))
        });
        let shared = matches!(record.binding, CredentialBinding::Password { .. });
        h_flex()
            .w_full()
            .py_3()
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(name)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(crate::credential_row::describe(record)),
                    )
                    .when(shared, |column| {
                        column.child(h_flex().flex_wrap().gap_1().children(detach))
                    }),
            )
            .child(self.actions(index, id, cx))
            .into_any_element()
    }

    /// Rename, copy the ID and delete.
    fn actions(&self, index: usize, id: CredentialId, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .child(
                Button::new(("vault-rename", index))
                    .small()
                    .ghost()
                    .icon(IconName::Pencil)
                    .tooltip("Rename")
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.start_rename(id, window, cx)),
                    ),
            )
            .child(
                Button::new(("vault-copy-id", index))
                    .small()
                    .ghost()
                    .icon(IconName::Copy)
                    .tooltip("Copy ID")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()))
                    }),
            )
            .child(
                Button::new(("vault-delete", index))
                    .small()
                    .ghost()
                    .icon(IconName::Trash)
                    .tooltip("Delete")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let deleted = this.service.delete(id);
                        this.operation(deleted, cx)
                    })),
            )
            .into_any_element()
    }

    /// The search and the button that adds a password.
    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .gap_2()
            .pt_2()
            .child(
                div().flex_1().child(
                    Input::new(&self.query)
                        .id("vault-search")
                        .small()
                        .prefix(gpui_kit::component::Icon::new(IconName::Search).small())
                        .cleanable(true),
                ),
            )
            .child(
                Button::new("vault-add-password")
                    .small()
                    .primary()
                    .icon(IconName::Plus)
                    .label("Add a password")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        crate::add_password::AddPassword::open(
                            this.service.clone(),
                            cx.weak_entity(),
                            window,
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }
}

impl Render for CredentialsTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.service.is_unlocked() {
            return form::section(
                "Saved credentials",
                [form::note(
                    "Unlock the vault on the Overview tab to see what it holds.",
                    cx,
                )],
                cx,
            );
        }
        let records = self.matching(cx);
        let mut rows: Vec<AnyElement> = records
            .iter()
            .enumerate()
            .map(|(index, record)| self.row(index, record, cx))
            .collect();
        if rows.is_empty() {
            let empty = if self.records.is_empty() {
                "Nothing saved yet. Add a password, or choose Remember when you sign in to a \
                 server."
            } else {
                "No saved credential has this name or server."
            };
            rows.push(form::note(empty, cx));
        }
        v_flex()
            .w_full()
            .child(self.toolbar(cx))
            .child(form::section("Saved credentials", rows, cx))
            .child(
                h_flex().pt_2().child(
                    Button::new("vault-refresh")
                        .small()
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            )
            .when_some(self.message.clone(), |tab, (message, error)| {
                tab.child(div().id("vault-credentials-message").child(if error {
                    form::error_text(message, cx)
                } else {
                    form::note(message, cx)
                }))
            })
            .into_any_element()
    }
}
