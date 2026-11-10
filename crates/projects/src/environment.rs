//! Observed environment settings and an explicit, source-bound tool mutation action.
mod source;
use crate::{
    Installed,
    languages::{LanguageChooser, LanguagesChanged},
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, img,
    prelude::*,
};
use nocterm_project_model::ProjectDefinition;
use nocterm_project_providers::{
    ContainerLocation, EnvironmentError, EnvironmentSnapshot, SetupUpdate,
};
use nocterm_project_service::EnvironmentOperation;
use nocterm_workspace::Workspace;
use source::CapturedEnvironment;

pub(crate) struct EnvironmentControls {
    installed: Installed,
    definition: ProjectDefinition,
    workspace: WeakEntity<Workspace>,
    authority: Result<CapturedEnvironment, EnvironmentError>,
    operation: Option<EnvironmentOperation>,
    languages: Entity<LanguageChooser>,
    snapshot: Option<EnvironmentSnapshot>,
    error: Option<EnvironmentError>,
    pending: bool,
    epoch: u64,
    progress: Option<SetupUpdate>,
    _subscriptions: Vec<Subscription>,
}
impl EnvironmentControls {
    pub(crate) fn new(
        installed: Installed,
        definition: ProjectDefinition,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let authority = workspace
            .upgrade()
            .ok_or(EnvironmentError::Unavailable)
            .and_then(|workspace| {
                CapturedEnvironment::capture(
                    &definition,
                    &installed,
                    workspace.read(cx),
                    window,
                    cx,
                )
            });
        let languages = cx.new(|cx| LanguageChooser::for_settings(window, cx));
        let subscriptions = vec![
            cx.on_release(|this, _| this.cancel()),
            cx.subscribe(&languages, |_, _, _: &LanguagesChanged, cx| cx.notify()),
        ];
        let mut controls = Self {
            installed,
            definition,
            workspace,
            authority,
            operation: None,
            languages,
            snapshot: None,
            error: None,
            pending: false,
            epoch: 0,
            progress: None,
            _subscriptions: subscriptions,
        };
        controls.run(false, cx);
        controls
    }
    fn admit(&self, cx: &mut App) -> Result<EnvironmentOperation, EnvironmentError> {
        let authority = self.authority.as_ref().map_err(Clone::clone)?;
        authority.validate(&self.installed, cx)?;
        self.installed.model().environment_operation(
            self.installed
                .environment()
                .cloned()
                .ok_or(EnvironmentError::Unavailable)?,
            authority.source(),
            cx,
        )
    }
    fn request(
        &self,
        install: bool,
        cx: &mut App,
    ) -> Result<
        (
            EnvironmentOperation,
            Vec<nocterm_project_providers::LanguageSelection>,
        ),
        EnvironmentError,
    > {
        self.admit(cx).and_then(|operation| {
            let selections = if install {
                self.languages.read(cx).selections(cx).map_err(|_| {
                    EnvironmentError::InvalidInput("check the selected language versions")
                })?
            } else {
                vec![]
            };
            if install && let Some(snapshot) = &self.snapshot {
                for selection in &selections {
                    snapshot.support(selection)?;
                }
            }
            Ok((operation, selections))
        })
    }
    fn run(&mut self, install: bool, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let request = self.request(install, cx);
        let (operation, selections) = match request {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.pending = true;
        self.error = None;
        self.progress = None;
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        self.operation = Some(operation.clone());
        self.languages
            .update(cx, |languages, cx| languages.configure(true, cx));
        cx.notify();
        let (send, receive) = async_channel::bounded(1);
        let latest = receive.clone();
        let job = cx.background_executor().spawn(async move {
            if install {
                operation
                    .install(
                        selections,
                        std::sync::Arc::new(move |update| {
                            if let Err(async_channel::TrySendError::Full(update)) =
                                send.try_send(update)
                            {
                                let _ = latest.try_recv();
                                let _ = send.try_send(update);
                            }
                        }),
                    )
                    .await
            } else {
                drop(send);
                operation.inspect().await
            }
        });
        cx.spawn(async move |owner, cx| {
            while let Ok(update) = receive.recv().await {
                let _ = owner.update(cx, |this, cx| {
                    if this.epoch == epoch
                        && this
                            .authority
                            .as_ref()
                            .is_ok_and(|source| source.validate(&this.installed, cx).is_ok())
                    {
                        this.progress = Some(update);
                        cx.notify();
                    }
                });
            }
            let result = job.await;
            let _ = owner.update(cx, |this, cx| this.complete(epoch, result, cx));
        })
        .detach();
    }
    fn complete(
        &mut self,
        epoch: u64,
        result: Result<EnvironmentSnapshot, EnvironmentError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        self.pending = false;
        self.operation = None;
        self.progress = None;
        self.languages
            .update(cx, |languages, cx| languages.configure(false, cx));
        let result = self
            .authority
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|source| source.validate(&self.installed, cx))
            .and(result);
        match result {
            Ok(snapshot) => {
                self.languages.update(cx, |languages, cx| {
                    languages.observe_environment(snapshot.clone(), cx)
                });
                self.snapshot = Some(snapshot);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    pub(crate) fn cancel(&mut self) {
        if let Some(operation) = self.operation.take() {
            operation.cancel();
        }
    }
    fn open_workspace(&self, window: &mut Window, cx: &mut App) {
        if let Some(workspace) = self.workspace.upgrade() {
            window.close_dialog(cx);
            let result = workspace.update(cx, |workspace, cx| {
                self.installed
                    .open(self.definition.id(), workspace, window, cx)
            });
            if let Err(error) = result {
                nocterm_ui::notice::error(
                    window,
                    cx,
                    "project-environment-connect",
                    "Could not open workspace",
                    error.to_string(),
                );
            }
        }
    }
    pub(crate) fn footer(&self, owner: WeakEntity<Self>, cx: &App) -> impl IntoElement + use<> {
        let actions = h_flex()
            .id("project-environment-actions")
            .test_support()
            .items_center()
            .gap_2()
            .child(crate::progress::render(
                "project-environment-progress",
                self.progress.as_ref(),
                self.pending,
                cx,
            ))
            .when(self.authority.is_ok(), |view| {
                view.child(
                    Button::new("project-environment-install")
                        .ghost()
                        .small()
                        .label("Install / update")
                        .disabled(
                            self.pending
                                || self.snapshot.is_none()
                                || self
                                    .languages
                                    .read(cx)
                                    .selections(cx)
                                    .is_ok_and(|selections| selections.is_empty()),
                        )
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.run(true, cx));
                        }),
                )
            });
        v_flex()
            .gap_2()
            .when(self.pending, |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Closing settings cancels remaining work. Completed tool changes stay.",
                        ),
                )
            })
            .when_some(self.error.as_ref(), |view, error| {
                view.child(
                    div()
                        .id("project-environment-error")
                        .test_support()
                        .role(gpui_kit::Role::Alert)
                        .aria_label(error_message(error))
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .max_h(gpui_kit::px(72.))
                        .overflow_y_scroll()
                        .child(error_message(error)),
                )
            })
            .child(actions)
    }
    fn identity(&self, cx: &App) -> impl IntoElement + use<> {
        let host = crate::location::host(self.definition.target());
        let summary = self
            .workspace
            .upgrade()
            .and_then(|workspace| crate::context::summary(workspace.read(cx), host.as_ref()?, cx));
        let name = summary
            .as_ref()
            .map(|summary| summary.name.clone())
            .unwrap_or_else(|| match host {
                Some(nocterm_project_model::HostLocator::Local) => "This computer".into(),
                Some(
                    nocterm_project_model::HostLocator::Ssh { target, .. }
                    | nocterm_project_model::HostLocator::SshDirect { target, .. },
                ) => target.to_string().into(),
                None => "Unavailable provider".into(),
            });
        h_flex()
            .gap_2()
            .min_w_0()
            .child(match summary.and_then(|summary| summary.icon) {
                Some(icon) => img(icon).size_4().into_any_element(),
                None => Icon::new(nocterm_ui::IconName::Server)
                    .small()
                    .into_any_element(),
            })
            .child(
                div()
                    .id("project-environment-host")
                    .test_support()
                    .aria_label(name.clone())
                    .text_sm()
                    .truncate()
                    .child(name),
            )
    }
    fn facts(&self, cx: &App) -> impl IntoElement + use<> {
        let location = ContainerLocation::from_target(self.definition.target()).ok();
        v_flex()
            .gap_1()
            .min_w_0()
            .when_some(location, |view, location| {
                let facts = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.container.as_ref());
                view.child(container_fact(&location, facts)).child(fact(
                    "image",
                    "Image",
                    facts
                        .and_then(|facts| facts.image.clone())
                        .unwrap_or_else(|| "Unavailable until connected".into()),
                ))
            })
            .when_some(self.snapshot.as_ref(), |view, snapshot| {
                view.child(fact(
                    "os",
                    "Operating system",
                    format!(
                        "{} {} · {}",
                        snapshot.os.name, snapshot.os.release, snapshot.os.architecture
                    ),
                ))
                .child(fact(
                    "user",
                    "User",
                    format!("{} (UID {})", snapshot.account.user, snapshot.account.uid),
                ))
            })
            .when(self.snapshot.is_none(), |view| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if self.pending {
                            "Reading the actual operating system and installed tool versions…"
                        } else {
                            "Operating system and tool versions have not been read."
                        }),
                )
            })
    }
}
fn container_fact(
    location: &ContainerLocation,
    facts: Option<&nocterm_project_providers::ContainerFacts>,
) -> impl IntoElement + use<> {
    let engine = match location.engine() {
        nocterm_containers::Engine::Docker => "Docker",
        nocterm_containers::Engine::Podman => "Podman",
    };
    let name = facts
        .and_then(|facts| facts.name.as_deref())
        .unwrap_or("Name unavailable");
    let full = format!("{engine} · {name} · {}", location.container());
    let summary = format!("{engine} · {name} · {}", &location.container()[..12]);
    fact_value(
        "container",
        "Container",
        full,
        div()
            .id("project-environment-container-summary")
            .test_support()
            .aria_label(summary.clone())
            .truncate()
            .child(summary),
    )
}
fn fact(id: &'static str, label: &'static str, value: String) -> impl IntoElement {
    fact_value(id, label, value.clone(), value)
}
fn fact_value(
    id: &'static str,
    label: &'static str,
    value: String,
    display: impl IntoElement,
) -> impl IntoElement {
    let tooltip = value.clone();
    v_flex()
        .gap_1()
        .min_w_0()
        .child(div().text_xs().font_medium().child(label))
        .child(
            div()
                .id(gpui_kit::SharedString::from(format!(
                    "project-environment-{id}"
                )))
                .test_support()
                .aria_label(value.clone())
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .text_sm()
                .w_full()
                .min_w_0()
                .truncate()
                .child(display),
        )
}
impl gpui_kit::Render for EnvironmentControls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().id("project-environment").test_support().w_full().min_w_0().gap_3()
            .child(h_flex().justify_between().child(div().text_sm().font_medium().child("Environment"))
                .child(Button::new("project-environment-refresh").ghost().xsmall().label("Refresh")
                    .disabled(self.pending || self.authority.is_err()).on_click(cx.listener(|this, _, _, cx| this.run(false, cx)))))
            .child(self.identity(cx)).child(self.facts(cx))
            .when(self.authority.is_ok(), |view| view.child(self.languages.clone())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if ContainerLocation::from_target(self.definition.target()).is_ok() {
                    "Install selected tools in this container for the current user. System packages may affect other workspaces using it."
                } else { "Install selected tools on this host for the current user. System packages may affect other workspaces on this host." }))
)
            .when(self.authority.is_err(), |view| view.child(Button::new("project-environment-connect")
                .ghost().small().label("Open workspace to connect").on_click(cx.listener(|this, _, window, cx| this.open_workspace(window, cx)))))
    }
}

fn error_message(error: &EnvironmentError) -> String {
    let mut message = error.to_string();
    if let EnvironmentError::PartialInstall { completed, .. } = error {
        message.push_str(&format!(
            ". Installed: {}. Other selections may need retry.",
            completed
                .iter()
                .map(|language| language.name())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!(". {cause}"));
        source = cause.source();
    }
    message
}
