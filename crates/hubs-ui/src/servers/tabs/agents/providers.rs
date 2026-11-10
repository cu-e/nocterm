//! The model providers an agent may use: the hub's, listed, and the account's own, added,
//! changed, tested and removed. A key is typed once; the hub keeps it and never shows it.

use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, TestSupportExt as _, Window,
    component::{
        Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_hub_agents_model::{Provider, ProviderDraft, ProviderId, ProviderKind, ProviderScope};
use nocterm_session::Secret;

use super::AgentsTab;
use crate::layout::{field, note};
use crate::messages::agents_problem;
use crate::servers::item;

/// What the person has typed for a provider of their own.
pub(super) struct ProviderForm {
    editing: Option<ProviderId>,
    kind: ProviderKind,
    name: Entity<InputState>,
    base_url: Entity<InputState>,
    models: Entity<InputState>,
    key: Entity<InputState>,
}

impl ProviderForm {
    fn new(provider: Option<&Provider>, window: &mut Window, cx: &mut Context<AgentsTab>) -> Self {
        let kind = provider.map_or(ProviderKind::default(), |provider| provider.kind);
        let mut input = |placeholder: &'static str, text: String, masked: bool| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked);
                input.set_value(text, window, cx);
                input
            })
        };
        Self {
            editing: provider.map(|provider| provider.id.clone()),
            kind,
            name: input(
                "Name",
                provider.map(|p| p.name.clone()).unwrap_or_default(),
                false,
            ),
            base_url: input(
                "Base URL",
                provider.map_or_else(
                    || kind.default_base_url().to_owned(),
                    |p| p.base_url.clone(),
                ),
                false,
            ),
            models: input(
                "Models, separated by commas",
                provider.map(|p| p.models.join(", ")).unwrap_or_default(),
                false,
            ),
            key: input(
                if provider.is_some() {
                    "API key (empty keeps the stored one)"
                } else {
                    "API key"
                },
                String::new(),
                true,
            ),
        }
    }

    fn draft(&self, cx: &gpui_kit::App) -> ProviderDraft {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let key = text(&self.key);
        ProviderDraft {
            kind: self.kind,
            name: text(&self.name),
            base_url: text(&self.base_url),
            models: text(&self.models)
                .split(',')
                .map(|model| model.trim().to_owned())
                .collect(),
            api_key: (!key.trim().is_empty()).then(|| Secret::new(key)),
        }
    }
}

impl AgentsTab {
    fn open_provider(
        &mut self,
        provider: Option<&Provider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.provider_form = Some(ProviderForm::new(provider, window, cx));
        self.agent_form = None;
        cx.notify();
    }

    fn save_provider(&mut self, cx: &mut Context<Self>) {
        let (Some(hub), Some(form)) = (self.hub.clone(), &self.provider_form) else {
            return;
        };
        let draft = form.draft(cx);
        let directory = self.service.read(cx).directory().clone();
        let call: nocterm_hub_agents_model::AgentsFuture<()> = match form.editing.clone() {
            Some(provider) => {
                let saving = directory.update_provider(&hub, &provider, draft);
                Box::pin(async move { saving.await.map(|_| ()) })
            }
            None => {
                let saving = directory.create_provider(&hub, draft);
                Box::pin(async move { saving.await.map(|_| ()) })
            }
        };
        self.write(call, cx);
    }

    fn delete_provider(&mut self, provider: &ProviderId, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let deleting = self
            .service
            .read(cx)
            .directory()
            .delete_provider(&hub, provider);
        self.write(deleting, cx);
    }

    /// Calls the provider from an agent worker and says what came of it.
    fn test_provider(&mut self, provider: &ProviderId, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let testing = self
            .service
            .read(cx)
            .directory()
            .test_provider(&hub, provider);
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = testing.await;
            this.update(cx, |tab, cx| {
                tab.problem = Some(match result {
                    Ok(test) if test.ok => format!(
                        "The provider answers. Models: {}",
                        if test.models.is_empty() {
                            "none listed".to_owned()
                        } else {
                            test.models.join(", ")
                        }
                    ),
                    Ok(test) => format!("The provider did not answer: {}", test.message),
                    Err(error) => agents_problem(&error),
                });
                cx.notify();
            })
            .ok();
        }));
    }
}

fn row(provider: &Provider, cx: &mut Context<AgentsTab>) -> gpui_kit::AnyElement {
    let scope = match provider.scope {
        ProviderScope::Hub => "the hub's",
        ProviderScope::Account => "your own",
    };
    let id = provider.id.clone();
    let mut actions = h_flex().gap_1().child(
        Button::new(item("server-provider-test", provider.id.as_str()))
            .small()
            .ghost()
            .label("Test")
            .on_click(cx.listener({
                let id = id.clone();
                move |tab, _: &ClickEvent, _, cx| tab.test_provider(&id, cx)
            })),
    );
    if provider.editable {
        let edit = provider.clone();
        actions = actions
            .child(
                Button::new(item("server-provider-edit", provider.id.as_str()))
                    .small()
                    .ghost()
                    .label("Edit")
                    .on_click(cx.listener(move |tab, _: &ClickEvent, window, cx| {
                        tab.open_provider(Some(&edit), window, cx);
                    })),
            )
            .child(
                Button::new(item("server-provider-delete", provider.id.as_str()))
                    .small()
                    .ghost()
                    .label("Delete")
                    .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
                        tab.delete_provider(&id, cx);
                    })),
            );
    }
    h_flex()
        .id(item("server-provider", provider.id.as_str()))
        .test_support()
        .gap_2()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().font_semibold().child(provider.name.clone()))
                .child(note(
                    format!(
                        "{scope} · {} · {}",
                        provider.base_url,
                        provider.models.join(", ")
                    ),
                    cx,
                )),
        )
        .child(actions)
        .into_any_element()
}

fn form(form: &ProviderForm, cx: &mut Context<AgentsTab>) -> gpui_kit::AnyElement {
    let kinds = h_flex().gap_1().children(
        [
            (ProviderKind::OpenAi, "OpenAI-compatible"),
            (ProviderKind::Anthropic, "Anthropic"),
        ]
        .map(|(kind, label)| {
            Button::new(gpui_kit::ElementId::Name(
                format!("server-provider-kind-{label}").into(),
            ))
            .small()
            .ghost()
            .label(label)
            .selected(form.kind == kind)
            .on_click(cx.listener(move |tab, _: &ClickEvent, _, cx| {
                if let Some(form) = tab.provider_form.as_mut() {
                    form.kind = kind;
                }
                cx.notify();
            }))
        }),
    );
    v_flex()
        .id("server-provider-form")
        .test_support()
        .gap_2()
        .child(field("Kind", kinds))
        .child(field("Name", Input::new(&form.name)))
        .child(field("Base URL", Input::new(&form.base_url)))
        .child(field("Models", Input::new(&form.models)))
        .child(field("API key", Input::new(&form.key)))
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("server-provider-cancel")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .on_click(cx.listener(|tab, _: &ClickEvent, _, cx| {
                            tab.provider_form = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("server-provider-save")
                        .small()
                        .primary()
                        .label("Save")
                        .on_click(cx.listener(|tab, _: &ClickEvent, _, cx| tab.save_provider(cx))),
                ),
        )
        .into_any_element()
}

/// The providers section: the providers, the form open on one, and adding one.
pub(super) fn render(
    providers: &[Provider],
    open: Option<&ProviderForm>,
    cx: &mut Context<AgentsTab>,
) -> gpui_kit::AnyElement {
    let rows: Vec<_> = providers.iter().map(|provider| row(provider, cx)).collect();
    v_flex()
        .id("server-providers")
        .test_support()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(div().font_semibold().child("Model providers"))
                .child(
                    Button::new("server-provider-new")
                        .small()
                        .label("Add your own")
                        .on_click(cx.listener(|tab, _: &ClickEvent, window, cx| {
                            tab.open_provider(None, window, cx);
                        })),
                ),
        )
        .when(rows.is_empty(), |list| {
            list.child(note(
                "No providers yet. Add your own, or ask the hub's administrator for one.",
                cx,
            ))
        })
        .children(rows)
        .children(open.map(|open| form(open, cx)))
        .into_any_element()
}
