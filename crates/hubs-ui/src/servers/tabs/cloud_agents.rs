//! The Cloud agents tab of a server: the agents of the account on that server, and creating,
//! editing and deleting the person's own agents. Agents of an organization or of the server are
//! managed in the web panel of the server, which this tab links to.
//!
//! The provider key of a personal agent never leaves the device unsealed and is never listed
//! again; the service seals the definition to the pool and the hub stays blind to it.

mod form;

use gpui_kit::{
    App, ClickEvent, Context, Entity, Subscription, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        dialog::DialogFooter,
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents::{CatalogStatus, CloudAgents};
use nocterm_cloud_agents_model::{
    AgentScope, CloudAgent, CloudAgentError, ProviderKind, SaveOutcome, web_panel_url,
};
use nocterm_hubs_model::HubId;

use crate::layout::{field, note, problem};
use crate::servers::item;
use form::{Form, Prefill};

/// The agents of the server that is shown, and the form of one of them.
pub(in crate::servers) struct CloudAgentsTab {
    service: Entity<CloudAgents>,
    hub: Option<HubId>,
    form: Option<Form>,
    working: Option<Task<()>>,
    problem: Option<String>,
    _service: Subscription,
}

impl CloudAgentsTab {
    pub(in crate::servers) fn new(service: Entity<CloudAgents>, cx: &mut Context<Self>) -> Self {
        Self {
            _service: cx.observe(&service, |_, _, cx| cx.notify()),
            service,
            hub: None,
            form: None,
            working: None,
            problem: None,
        }
    }

    /// Shows the agents of `hub`; another server closes the form.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() != Some(hub) {
            self.hub = Some(hub.clone());
            self.form = None;
            self.working = None;
            self.problem = None;
            self.service
                .update(cx, |service, cx| service.track(hub, cx));
            cx.notify();
        }
    }

    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.form = None;
        self.working = None;
    }

    fn open_form(&mut self, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.problem = None;
        self.form = Some(Form::new(prefill, window, cx));
        let pools = self.service.update(cx, |service, cx| service.pools(&hub, cx));
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = pools.await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(pools) => {
                        if let Some(form) = &mut tab.form {
                            form.offer_pools(pools);
                        }
                    }
                    Err(error) => tab.problem = Some(describe(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        self.working = None;
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(hub), Some(form)) = (self.hub.clone(), self.form.as_mut()) else {
            return;
        };
        let existing = form.existing();
        match form.draft(window, cx) {
            Ok(draft) => self.save(hub, existing, draft, cx),
            Err(error) => {
                self.problem = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn save(
        &mut self,
        hub: HubId,
        existing: Option<(nocterm_cloud_agents_model::AgentId, u64)>,
        draft: nocterm_cloud_agents_model::PersonalAgentDraft,
        cx: &mut Context<Self>,
    ) {
        self.problem = None;
        let saving = self.service.update(cx, |service, cx| {
            service.save_personal(&hub, existing, draft.clone(), cx)
        });
        self.working = Some(cx.spawn(async move |this, cx| {
            let outcome = saving.await;
            this.update(cx, |tab, cx| {
                match outcome {
                    Ok(SaveOutcome::Saved(_)) => tab.form = None,
                    Ok(SaveOutcome::NeedsPoolTrust(offer)) => {
                        if let Some(form) = &mut tab.form {
                            form.wait_for_trust(draft, offer);
                        }
                    }
                    Err(error) => tab.problem = Some(describe(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The person compared the fingerprint: pins the pool and saves again.
    fn trust_and_save(&mut self, cx: &mut Context<Self>) {
        let (Some(hub), Some(form)) = (self.hub.clone(), self.form.as_mut()) else {
            return;
        };
        let existing = form.existing();
        let Some((draft, offer)) = form.take_waiting() else {
            return;
        };
        let pinning = self
            .service
            .update(cx, |service, cx| service.pin_pool(&hub, &offer, cx));
        self.working = Some(cx.spawn(async move |this, cx| {
            let pinned = pinning.await;
            this.update(cx, |tab, cx| match pinned {
                Ok(()) => tab.save(hub, existing, draft, cx),
                Err(error) => {
                    tab.problem = Some(describe(&error));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn delete(&mut self, agent: &CloudAgent, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.problem = None;
        let deleting = self
            .service
            .update(cx, |service, cx| {
                service.delete_personal(&hub, &agent.id, agent.version, cx)
            });
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = deleting.await;
            this.update(cx, |tab, cx| {
                if let Err(error) = result {
                    tab.problem = Some(describe(&error));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn confirm_delete(
        view: Entity<Self>,
        agent: CloudAgent,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, chosen) = (view.clone(), agent.clone());
            let confirm = Button::new("cloud-agent-delete-confirm")
                .danger()
                .label("Delete")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    view.update(cx, |tab, cx| tab.delete(&chosen, cx));
                });
            let keep = Button::new("cloud-agent-delete-keep")
                .primary()
                .label("Keep it")
                .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                });
            alert
                .title(format!("Delete {}?", agent.name))
                .description("The agent and its sealed provider key are removed from the server.")
                .on_ok(|_, _, _| true)
                .footer(DialogFooter::new().child(confirm).child(keep))
        });
    }
}

fn describe(error: &CloudAgentError) -> String {
    error.to_string()
}

impl Render for CloudAgentsTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(hub) = self.hub.clone() else {
            return div().into_any_element();
        };
        let view = cx.entity();
        let key = hub.address().to_string();
        let service = self.service.read(cx);
        let status = service.status(&hub);
        let agents: Vec<CloudAgent> = service.agents(&hub).to_vec();
        let web = web_panel_url(&hub);
        let header = v_flex()
            .gap_2()
            .child(note(
                "Your own agents live here. Agents of an organization or of the server are managed in the web panel of the server.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(item("cloud-agents-new", &key))
                            .primary()
                            .label("New personal agent")
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |tab, cx| tab.open_form(None, window, cx));
                                }
                            }),
                    )
                    .child(
                        Button::new(item("cloud-agents-web", &key))
                            .label("Manage agents on the web")
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                cx.open_url(&web)
                            }),
                    ),
            );
        let body = match status {
            Some(CatalogStatus::Unsupported) => div()
                .id(item("cloud-agents-unsupported", &key))
                .test_support()
                .child(note("This server does not offer Cloud agents.", cx))
                .into_any_element(),
            None | Some(CatalogStatus::Loading) => div()
                .id(item("cloud-agents-loading", &key))
                .test_support()
                .child(note("Reading the agents of your account…", cx))
                .into_any_element(),
            Some(_) if agents.is_empty() => div()
                .id(item("cloud-agents-empty", &key))
                .test_support()
                .child(note("No Cloud agents yet.", cx))
                .into_any_element(),
            Some(_) => v_flex()
                .gap_2()
                .children(agents.iter().map(|agent| row(agent, &key, &view, cx)))
                .into_any_element(),
        };
        v_flex()
            .id(item("cloud-agents-tab", &key))
            .test_support()
            .gap_3()
            .child(header)
            .children(self.problem.clone().map(|text| {
                div()
                    .id(item("cloud-agents-problem", &key))
                    .test_support()
                    .child(problem(text, cx))
            }))
            .children(self.form.as_ref().map(|form| render_form(form, &key, &view, cx)))
            .child(body)
            .into_any_element()
    }
}

fn scope_text(scope: &AgentScope) -> String {
    match scope {
        AgentScope::Personal => "Personal".to_owned(),
        AgentScope::Org { name, .. } => name.clone(),
        AgentScope::Server => "Server".to_owned(),
    }
}

/// One agent: its name, owner, model and status, and for the person's own the buttons that edit
/// and delete it.
fn row(agent: &CloudAgent, key: &str, view: &Entity<CloudAgentsTab>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let personal = agent.scope == AgentScope::Personal;
    let id = agent.id.to_string();
    let (edit_view, edit_agent) = (view.clone(), agent.clone());
    let (delete_view, delete_agent) = (view.clone(), agent.clone());
    h_flex()
        .id(item(&format!("cloud-agent-{id}"), key))
        .test_support()
        .gap_3()
        .p_3()
        .items_center()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().font_semibold().truncate().child(agent.name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} - {} - {}",
                            scope_text(&agent.scope),
                            agent.primary_model().unwrap_or("no model"),
                            agent.status.label()
                        )),
                ),
        )
        .when(personal, |line| {
            line.child(
                Button::new(item(&format!("cloud-agent-edit-{id}"), key))
                    .label("Edit")
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let prefill = Prefill {
                            id: edit_agent.id.clone(),
                            version: edit_agent.version,
                            name: edit_agent.name.clone(),
                            model: edit_agent.primary_model().unwrap_or_default().to_owned(),
                            provider: ProviderKind::from_id(&edit_agent.provider),
                        };
                        edit_view.update(cx, |tab, cx| tab.open_form(Some(prefill), window, cx));
                    }),
            )
            .child(
                Button::new(item(&format!("cloud-agent-delete-{id}"), key))
                    .danger()
                    .label("Delete")
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        CloudAgentsTab::confirm_delete(
                            delete_view.clone(),
                            delete_agent.clone(),
                            window,
                            cx,
                        );
                    }),
            )
        })
}

fn render_form(
    form: &Form,
    key: &str,
    view: &Entity<CloudAgentsTab>,
    cx: &mut Context<CloudAgentsTab>,
) -> impl IntoElement {
    let [name, endpoint, model, secret] = form.inputs();
    let theme = cx.theme().clone();
    let provider_buttons = ProviderKind::ALL.map(|kind| {
        let view = view.clone();
        Button::new(item(&format!("cloud-agent-provider-{}", kind.id()), key))
            .label(kind.label())
            .when(form.provider() == kind, |button| button.primary())
            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                view.update(cx, |tab, cx| {
                    if let Some(form) = &mut tab.form {
                        form.set_provider(kind);
                    }
                    cx.notify();
                });
            })
    });
    let offline_view = view.clone();
    let offline = Checkbox::new(item("cloud-agent-offline", key))
        .label("Agents work when I'm offline")
        .checked(form.work_offline())
        .on_click(move |checked: &bool, _: &mut Window, cx: &mut App| {
            let on = *checked;
            offline_view.update(cx, |tab, cx| {
                if let Some(form) = &mut tab.form {
                    form.set_work_offline(on);
                }
                cx.notify();
            });
        });
    let pools = form.work_offline().then(|| {
        let buttons = form.pools().iter().filter(|pool| pool.accepts_personal).map(|pool| {
            let (view, id) = (view.clone(), pool.pool.clone());
            Button::new(item(&format!("cloud-agent-pool-{}", pool.pool), key))
                .label(pool.name.clone())
                .when(form.chosen_pool() == Some(&pool.pool), |button| button.primary())
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    view.update(cx, |tab, cx| {
                        if let Some(form) = &mut tab.form {
                            form.choose_pool(id.clone());
                        }
                        cx.notify();
                    });
                })
        });
        field("Pool", h_flex().gap_2().children(buttons))
    });
    let submit_view = view.clone();
    let cancel_view = view.clone();
    let trust = form.waiting_offer().map(|offer| {
        let view = view.clone();
        v_flex()
            .id(item("cloud-agent-trust", key))
            .test_support()
            .gap_2()
            .child(note(
                format!(
                    "Trust pool \"{}\"? Compare this fingerprint with the one its owner publishes before you do.",
                    offer.name
                ),
                cx,
            ))
            .child(
                div()
                    .id(item("cloud-agent-trust-fingerprint", key))
                    .test_support()
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .child(offer.fingerprint.clone()),
            )
            .child(
                Button::new(item("cloud-agent-trust-confirm", key))
                    .primary()
                    .label("Trust pool and save")
                    .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        view.update(cx, |tab, cx| tab.trust_and_save(cx));
                    }),
            )
    });
    v_flex()
        .id(item("cloud-agent-form", key))
        .test_support()
        .gap_3()
        .p_3()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .child(field("Name", div().id(item("cloud-agent-name", key)).test_support().child(Input::new(name))))
        .child(field("Provider", h_flex().gap_2().children(provider_buttons)))
        .child(field("Endpoint", div().id(item("cloud-agent-endpoint", key)).test_support().child(Input::new(endpoint))))
        .child(field("Model", div().id(item("cloud-agent-model", key)).test_support().child(Input::new(model))))
        .child(field("Provider key", div().id(item("cloud-agent-key", key)).test_support().child(Input::new(secret))))
        .child(note(
            if form.is_edit() {
                "Saving replaces the whole definition, so enter the endpoint and the key again. The key is stored encrypted and is never shown."
            } else {
                "The key is stored encrypted and is never shown again. Use a provider key with a spending limit."
            },
            cx,
        ))
        .child(offline)
        .child(note(
            "On: the agent keeps working on a server pool while this computer is off, which means the pool's host can use the key. Off: it runs only on this computer.",
            cx,
        ))
        .children(pools)
        .children(trust)
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new(item("cloud-agent-cancel", key))
                        .label("Cancel")
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            cancel_view.update(cx, |tab, cx| tab.close_form(cx));
                        }),
                )
                .child(
                    Button::new(item("cloud-agent-save", key))
                        .primary()
                        .label("Save")
                        .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            submit_view.update(cx, |tab, cx| tab.submit(window, cx));
                        }),
                ),
        )
}
