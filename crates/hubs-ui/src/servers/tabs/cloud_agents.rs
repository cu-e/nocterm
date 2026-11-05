//! The Cloud agents tab of a server: the agents of the account on that server, and creating,
//! editing and deleting the person's own agents. Agents of an organization or of the server are
//! managed in the web panel of the server, which this tab links to.
//!
//! The provider key of a personal agent never leaves the device unsealed and is never listed
//! again; the service seals the definition to the pool and the hub stays blind to it.

mod form;
mod pinned;
mod usage;

use gpui_kit::{
    App, ClickEvent, Context, Entity, Subscription, Task, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        h_flex, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_cloud_agents::{CatalogStatus, CloudAgents};
use nocterm_cloud_agents_model::{
    AgentScope, CloudAgent, CloudAgentError, DraftError, PinnedPool, PoolId, ProviderKind,
    SaveOutcome,
};
use nocterm_hubs_model::HubId;

use crate::layout::{note, problem};
use crate::servers::item;
use form::{Form, Prefill, PrefillModels, render_form};
use pinned::pinned_section;
use usage::{USAGE_DAYS, UsageTable};

enum Opening {
    Closed,
    Loading { _work: Task<()> },
    Missing(Prefill),
}

/// The agents of the server that is shown, and the form of one of them.
pub(in crate::servers) struct CloudAgentsTab {
    service: Entity<CloudAgents>,
    hub: Option<HubId>,
    form: Option<Form>,
    working: Option<Task<()>>,
    opening: Opening,
    form_epoch: u64,
    problem: Option<String>,
    pinned: Vec<PinnedPool>,
    pinning: Option<Task<()>>,
    usage: UsageTable,
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
            opening: Opening::Closed,
            form_epoch: 0,
            problem: None,
            pinned: Vec::new(),
            pinning: None,
            usage: UsageTable::default(),
        }
    }

    /// Shows the agents of `hub`; another server closes the form.
    pub(in crate::servers) fn show(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.hub.as_ref() != Some(hub) {
            self.hub = Some(hub.clone());
            self.reset_form();
            self.problem = None;
            self.pinned.clear();
            self.usage.clear();
            self.service
                .update(cx, |service, cx| service.track(hub, cx));
            self.refresh_pinned(cx);
            self.refresh_usage(cx);
            cx.notify();
        }
    }

    pub(in crate::servers) fn hide(&mut self) {
        self.hub = None;
        self.pinning = None;
        self.usage.clear();
        self.reset_form();
    }

    fn reset_form(&mut self) {
        self.form_epoch = self.form_epoch.wrapping_add(1);
        self.form = None;
        self.working = None;
        self.opening = Opening::Closed;
    }

    fn refresh_pinned(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let listing = self
            .service
            .update(cx, |service, cx| service.pinned_pools(&hub, cx));
        self.pinning = Some(cx.spawn(async move |this, cx| {
            let result = listing.await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(pinned) => tab.pinned = pinned,
                    Err(error) => tab.problem = Some(describe(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Reads the per-model usage. A hub that cannot report it simply shows no table.
    fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let reading = self.service.update(cx, |service, cx| {
            service.personal_usage(&hub, USAGE_DAYS, cx)
        });
        let task = cx.spawn(async move |this, cx| {
            let result = reading.await;
            this.update(cx, |tab, cx| {
                if let Ok(rows) = result {
                    tab.usage.show(rows);
                }
                cx.notify();
            })
            .ok();
        });
        self.usage.reading(task);
    }

    fn forget_pool(&mut self, pool: &PoolId, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        self.problem = None;
        let forgetting = self
            .service
            .update(cx, |service, cx| service.forget_pool(&hub, pool, cx));
        self.pinning = Some(cx.spawn(async move |this, cx| {
            let result = forgetting.await;
            this.update(cx, |tab, cx| {
                match result {
                    Ok(()) => tab.refresh_pinned(cx),
                    Err(error) => tab.problem = Some(describe(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn open_form(&mut self, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_form();
        self.problem = None;
        let epoch = self.form_epoch;
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let mut prefill = match prefill {
            Some(prefill) if !prefill.work_offline => prefill,
            other => {
                self.install_form(other, window, cx);
                return;
            }
        };
        let reading = self.service.update(cx, |service, cx| {
            service.personal_models(&hub, &prefill.id, cx)
        });
        let work = cx.spawn_in(window, async move |this, cx| {
            let result = reading.await;
            let _ = this.update_in(cx, |tab, window, cx| {
                if tab.hub.as_ref() != Some(&hub) || tab.form_epoch != epoch {
                    return;
                }
                tab.opening = Opening::Closed;
                match result {
                    Ok(models) => {
                        prefill.models = PrefillModels::Canonical(models);
                        tab.install_form(Some(prefill), window, cx);
                    }
                    Err(CloudAgentError::Invalid(DraftError::Key)) => {
                        tab.opening = Opening::Missing(prefill);
                        tab.problem = Some("No saved provider key was found. You can enter a replacement key and review the models from the agent listing.".to_owned());
                        cx.notify();
                    }
                    Err(error) => {
                        tab.problem = Some(describe(&error));
                        cx.notify();
                    }
                }
            });
        });
        self.opening = Opening::Loading { _work: work };
        cx.notify();
    }

    fn replace_missing_key(&mut self, epoch: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.form_epoch != epoch || !matches!(self.opening, Opening::Missing(_)) {
            return;
        }
        let Opening::Missing(mut prefill) = std::mem::replace(&mut self.opening, Opening::Closed)
        else {
            return;
        };
        let models = match prefill.models {
            PrefillModels::Listed(models) => models,
            _ => return,
        };
        prefill.models = PrefillModels::Missing(models);
        self.install_form(Some(prefill), window, cx);
    }

    fn install_form(
        &mut self,
        prefill: Option<Prefill>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let epoch = self.form_epoch;
        self.problem = None;
        self.form = Some(Form::new(prefill, window, cx));
        let pools = self
            .service
            .update(cx, |service, cx| service.pools(&hub, cx));
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = pools.await;
            this.update(cx, |tab, cx| {
                if tab.form_epoch != epoch || tab.hub.as_ref() != Some(&hub) {
                    return;
                }
                tab.working = None;
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
        self.reset_form();
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.working.is_some()
            || self
                .form
                .as_ref()
                .is_some_and(|form| form.waiting_offer().is_some())
        {
            return;
        }
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
        let epoch = self.form_epoch;
        self.problem = None;
        let saving = self.service.update(cx, |service, cx| {
            service.save_personal(&hub, existing, draft.clone(), cx)
        });
        self.working = Some(cx.spawn(async move |this, cx| {
            let outcome = saving.await;
            this.update(cx, |tab, cx| {
                if tab.form_epoch != epoch || tab.hub.as_ref() != Some(&hub) {
                    return;
                }
                tab.working = None;
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
        if self.working.is_some() {
            return;
        }
        let (Some(hub), Some(form)) = (self.hub.clone(), self.form.as_mut()) else {
            return;
        };
        let epoch = self.form_epoch;
        let existing = form.existing();
        let Some((draft, offer)) = form.take_waiting() else {
            return;
        };
        let pinning = self
            .service
            .update(cx, |service, cx| service.pin_pool(&hub, &offer, cx));
        self.working = Some(cx.spawn(async move |this, cx| {
            let pinned = pinning.await;
            this.update(cx, |tab, cx| {
                if tab.form_epoch != epoch || tab.hub.as_ref() != Some(&hub) {
                    return;
                }
                tab.working = None;
                match pinned {
                    Ok(()) => tab.save(hub, existing, draft, cx),
                    Err(error) => {
                        tab.problem = Some(describe(&error));
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    fn delete(&mut self, agent: &CloudAgent, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let epoch = self.form_epoch;
        self.problem = None;
        let deleting = self.service.update(cx, |service, cx| {
            service.delete_personal(&hub, &agent.id, agent.version, cx)
        });
        self.working = Some(cx.spawn(async move |this, cx| {
            let result = deleting.await;
            this.update(cx, |tab, cx| {
                if tab.form_epoch != epoch || tab.hub.as_ref() != Some(&hub) {
                    return;
                }
                tab.working = None;
                if let Err(error) = result {
                    tab.problem = Some(describe(&error));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn confirm_delete(view: Entity<Self>, agent: CloudAgent, window: &mut Window, cx: &mut App) {
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

fn pending_form(
    opening: &Opening,
    epoch: u64,
    key: &str,
    view: &Entity<CloudAgentsTab>,
    cx: &App,
) -> Option<gpui_kit::AnyElement> {
    let view = view.clone();
    match opening {
        Opening::Closed => None,
        Opening::Loading { .. } => Some(
            h_flex()
                .id(item("cloud-agent-models-loading", key))
                .test_support()
                .gap_2()
                .child(note("Reading the agent's saved models…", cx))
                .child(
                    Button::new(item("cloud-agent-models-cancel", key))
                        .label("Cancel")
                        .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            view.update(cx, |tab, cx| {
                                if tab.form_epoch == epoch {
                                    tab.close_form(cx);
                                }
                            });
                        }),
                )
                .into_any_element(),
        ),
        Opening::Missing(_) => Some(
            Button::new(item("cloud-agent-replace-key", key))
                .label("Enter replacement key")
                .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    view.update(cx, |tab, cx| tab.replace_missing_key(epoch, window, cx));
                })
                .into_any_element(),
        ),
    }
}

fn catalog_header(
    key: &str,
    web: String,
    view: &Entity<CloudAgentsTab>,
    cx: &App,
) -> impl IntoElement {
    v_flex()
            .gap_2()
            .child(note(
                "Your own agents live here. Agents of an organization or of the server are managed in the web panel of the server.",
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(item("cloud-agents-new", key))
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
                        Button::new(item("cloud-agents-web", key))
                            .label("Manage agents on the web")
                            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                cx.open_url(&web)
                            }),
                    ),
            )
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
        let web = service.web_panel_url(&hub);
        let header = catalog_header(&key, web, &view, cx);
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
            .children(pending_form(
                &self.opening,
                self.form_epoch,
                &key,
                &view,
                cx,
            ))
            .children(self.form.as_ref().map(|form| {
                render_form(
                    form,
                    self.form_epoch,
                    self.working.is_some(),
                    &key,
                    &view,
                    cx,
                )
            }))
            .child(body)
            .children(self.usage.render(&key, cx))
            .children(pinned_section(&self.pinned, &key, &view, cx))
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
                            models: PrefillModels::Listed(edit_agent.models.clone()),
                            provider: ProviderKind::from_id(&edit_agent.provider),
                            work_offline: edit_agent.placement()
                                == nocterm_cloud_agents_model::Placement::Pool,
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
