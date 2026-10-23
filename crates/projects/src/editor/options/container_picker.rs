//! Original-host container discovery, filtering and exact resource selection.
use super::container_row;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement,
    Render, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_containers::Engine;
use nocterm_project_model::HostLocator;
use nocterm_project_providers::{EngineInventory, ProjectPreparation, ProjectSetupError};
use nocterm_workspace_ports::ProjectCreationSource;
use std::sync::Arc;
pub(super) enum ContainerPickerEvent {
    Changed,
    Inventory,
}
pub(super) struct ContainerPicker {
    source: ProjectCreationSource,
    preparation: Option<Arc<dyn ProjectPreparation>>,
    inventory: Vec<EngineInventory>,
    loading: bool,
    error: Option<ProjectSetupError>,
    search: Entity<InputState>,
    selected: Option<(Engine, String)>,
    locked: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ContainerPickerEvent> for ContainerPicker {}
impl ContainerPicker {
    pub(super) fn new(
        source: ProjectCreationSource,
        preparation: Option<Arc<dyn ProjectPreparation>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Name, image, container ID or engine")
        });
        let subscription = cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let mut this = Self {
            source,
            preparation,
            inventory: vec![],
            loading: false,
            error: None,
            search,
            selected: None,
            locked: false,
            _subscriptions: vec![subscription],
        };
        this.discover(cx);
        this
    }
    pub(super) fn inventory(&self) -> &[EngineInventory] {
        &self.inventory
    }
    pub(super) fn selected(&self) -> Option<(Engine, String)> {
        self.selected.clone()
    }
    pub(super) fn focus(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
    pub(super) fn lock(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.locked = locked;
        cx.notify();
    }
    fn matches(
        &self,
        engine: Engine,
        container: &nocterm_containers::Container,
        images: &[nocterm_containers::Image],
        cx: &App,
    ) -> bool {
        let query = self.search.read(cx).value().trim().to_ascii_lowercase();
        let engine = match engine {
            Engine::Docker => "docker",
            Engine::Podman => "podman",
        };
        [
            container.name.as_str(),
            &container.id,
            &container.image,
            engine,
        ]
        .iter()
        .any(|text| text.to_ascii_lowercase().contains(&query))
            || images
                .iter()
                .filter(|image| image.id == container.image)
                .any(|image| {
                    format!(
                        "{}:{}",
                        image.repository.as_deref().unwrap_or(""),
                        image.tag.as_deref().unwrap_or("")
                    )
                    .to_ascii_lowercase()
                    .contains(&query)
                })
    }
    fn discover(&mut self, cx: &mut Context<Self>) {
        let Some(preparation) = self.preparation.clone() else {
            return;
        };
        let Some(exec) = self.source.executor() else {
            return;
        };
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(preparation.discover(exec))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(inventory) => {
                        this.inventory = inventory;
                    }
                    Err(error) => this.error = Some(error),
                };
                cx.emit(ContainerPickerEvent::Inventory);
                cx.notify();
            });
        })
        .detach();
    }
    fn engine_inventory(
        &self,
        inventory: &EngineInventory,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let engine = inventory.engine;
        let label = match engine {
            Engine::Docker => "Docker",
            Engine::Podman => "Podman",
        };
        v_flex()
            .gap_1()
            .when_some(inventory.containers.as_ref().err(), |view, error| {
                view.child(
                    div()
                        .id(gpui_kit::SharedString::from(format!(
                            "project-container-engine-error-{label}"
                        )))
                        .test_support()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{label}: {error}")),
                )
            })
            .when_some(inventory.containers.as_ref().ok(), |view, containers| {
                view.when(containers.is_empty(), |view| {
                    view.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{label}: no containers")),
                    )
                })
                .children(
                    containers
                        .iter()
                        .filter(|container| {
                            self.matches(
                                engine,
                                container,
                                inventory.images.as_deref().unwrap_or(&[]),
                                cx,
                            )
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .map(|container| {
                            self.container_row(
                                engine,
                                label,
                                container,
                                inventory.images.as_deref().unwrap_or(&[]),
                                cx,
                            )
                        }),
                )
            })
    }
    fn selected_summary(&self, engine: Engine, id: &str) -> impl IntoElement + use<> {
        let name = self
            .inventory
            .iter()
            .filter(|inventory| inventory.engine == engine)
            .filter_map(|inventory| inventory.containers.as_ref().ok())
            .flatten()
            .find(|container| container.id == id)
            .map(|container| container.name.as_str())
            .unwrap_or("Unavailable container");
        let full = format!("Selected {name} · {engine:?}: {id}");
        let tooltip = full.clone();
        let compact = id.chars().take(12).collect::<String>();
        h_flex()
            .id("project-container-selected")
            .test_support()
            .w_full()
            .min_w_0()
            .gap_2()
            .aria_label(full)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .child(format!("Selected: {name}")),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .child(format!("{engine:?} · {compact}")),
            )
    }
    fn container_row(
        &self,
        engine: Engine,
        label: &str,
        container: &nocterm_containers::Container,
        images: &[nocterm_containers::Image],
        cx: &mut Context<Self>,
    ) -> Button {
        let id = container.id.clone();
        let selected = self
            .selected
            .as_ref()
            .is_some_and(|(selected_engine, selected_id)| {
                *selected_engine == engine && selected_id == &id
            });
        container_row::ContainerRow::new(container, images, label, selected)
            .render(cx)
            .disabled(self.locked)
            .on_click(cx.listener(move |this, _, _, cx| {
                let selected = (engine, id.clone());
                if this.selected.as_ref() != Some(&selected) {
                    this.selected = Some(selected);
                    cx.emit(ContainerPickerEvent::Changed);
                    cx.notify();
                }
            }))
    }
}
impl Render for ContainerPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (window.viewport_size().height - px(520.))
            .max(px(80.))
            .min(px(200.));
        let matches = self
            .inventory
            .iter()
            .filter_map(|inventory| {
                inventory
                    .containers
                    .as_ref()
                    .ok()
                    .map(|containers| (inventory, containers))
            })
            .any(|(inventory, containers)| {
                containers.iter().any(|container| {
                    self.matches(
                        inventory.engine,
                        container,
                        inventory.images.as_deref().unwrap_or(&[]),
                        cx,
                    )
                })
            });
        v_flex().gap_1().w_full().min_w_0()
            .child(h_flex().justify_between().child(div().text_sm().font_medium().child(if matches!(self.source.parent().host(), HostLocator::Local) { "Containers on this computer" } else { "Containers on this server" }))
                .child(Button::new("project-container-refresh").ghost().xsmall().label("Refresh").disabled(self.loading || self.locked).on_click(cx.listener(|this, _, _, cx| this.discover(cx)))))
            .child(div().id("project-container-search").test_support().child(Input::new(&self.search).id("input").aria_label("Search containers").small().disabled(self.locked)))
            .when_some(self.selected.as_ref(), |view, (engine, id)| view.child(self.selected_summary(*engine, id)))
            .when(self.loading, |view| view.child(div().text_xs().text_color(cx.theme().muted_foreground).child("Loading Docker and Podman containers…")))
            .when(self.source.executor().is_none(), |view| view.child(div().text_xs().text_color(cx.theme().muted_foreground).child("This session does not offer command execution. Reconnect to list containers.")))
            .when(!matches && !self.loading && !self.search.read(cx).value().is_empty(), |view| view.child(div().id("project-container-no-matches").test_support().text_xs().child("No containers match this search. Clear the search to see all containers.")))
            .child(v_flex().id("project-container-list").test_support().w_full().min_w_0().gap_1().max_h(height).overflow_y_scroll().children(self.inventory.iter().map(|inventory| self.engine_inventory(inventory, cx))))
            .when_some(self.error.as_ref(), |view, error| view.child(nocterm_ui::form::error_text(error.to_string(), cx)))
    }
}
