//! Image draft, immediate local filtering and captured-host registry search.
use super::registry_search::{RegistrySearchState, SearchKey, SearchStatus};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_containers::{Engine, Image};
use nocterm_project_providers::{
    EngineInventory, ImageSearch, ImageSearchQuery, RegistryImage, WorkspaceImage,
};
use nocterm_workspace_ports::ProjectCreationSource;
use std::{sync::Arc, time::Duration};

pub(super) struct ImageChanged;
impl EventEmitter<ImageChanged> for ImagePicker {}
#[derive(Default)]
struct LocalImages {
    images: Vec<Image>,
    error: Option<String>,
}
pub(super) struct ImagePicker {
    source: ProjectCreationSource,
    search: Option<Arc<dyn ImageSearch>>,
    engine: Engine,
    image: WorkspaceImage,
    input: Entity<InputState>,
    local: LocalImages,
    registry: RegistrySearchState,
    results_source: usize,
    enabled: bool,
    locked: bool,
    _subscriptions: Vec<Subscription>,
}
impl ImagePicker {
    pub(super) fn new(
        source: ProjectCreationSource,
        search: Option<Arc<dyn ImageSearch>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Repository or full image reference, e.g. debian:13")
        });
        let changed = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
                cx.emit(ImageChanged);
            }
        });
        Self {
            source,
            search,
            engine: Engine::Docker,
            image: WorkspaceImage::Ubuntu,
            input,
            local: LocalImages::default(),
            registry: RegistrySearchState::default(),
            results_source: 0,
            enabled: false,
            locked: false,
            _subscriptions: vec![changed],
        }
    }
    pub(super) fn focus(&self, cx: &App) -> gpui_kit::FocusHandle {
        use gpui_kit::Focusable as _;
        self.input.focus_handle(cx)
    }
    pub(super) fn image(&self, cx: &App) -> WorkspaceImage {
        if self.is_custom() {
            WorkspaceImage::Custom(self.input.read(cx).value().to_string())
        } else {
            self.image.clone()
        }
    }
    pub(super) fn is_custom(&self) -> bool {
        matches!(self.image, WorkspaceImage::Custom(_))
    }
    pub(super) fn configure(
        &mut self,
        engine: Engine,
        enabled: bool,
        locked: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.engine != engine || self.enabled != enabled || self.locked != locked;
        self.engine = engine;
        self.enabled = enabled;
        self.locked = locked;
        if changed {
            self.registry.invalidate(true);
            self.refresh(cx);
        }
        cx.notify();
    }
    pub(super) fn inventory(&mut self, inventories: &[EngineInventory]) {
        self.local = match inventories
            .iter()
            .find(|inventory| inventory.engine == self.engine)
            .map(|inventory| &inventory.images)
        {
            Some(Ok(images)) => LocalImages {
                images: images.clone(),
                error: None,
            },
            Some(Err(error)) => LocalImages {
                images: vec![],
                error: Some(error.to_string()),
            },
            None => LocalImages::default(),
        };
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let draft = self.input.read(cx).value().to_string();
        if !self.enabled || self.locked || !self.is_custom() {
            self.registry.invalidate(true);
            cx.notify();
            return;
        }
        if self.registry.was_selected(&draft) {
            cx.notify();
            return;
        }
        let (Some(search), Some(exec), Ok(query)) = (
            self.search.clone(),
            self.source.executor(),
            ImageSearchQuery::parse(&draft),
        ) else {
            self.registry.invalidate(true);
            cx.notify();
            return;
        };
        let key = SearchKey::new(
            format!("{:?}:{:p}", self.source.parent().host(), Arc::as_ptr(&exec)),
            self.engine,
            query.as_str(),
        );
        let Some((generation, cancel)) =
            self.registry.prepare(&key, cx.background_executor().now())
        else {
            cx.notify();
            return;
        };
        let engine = self.engine;
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if cancel.is_cancelled() {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(search.search(exec, engine, query, cancel))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.registry
                    .complete(generation, key, result, cx.background_executor().now());
                cx.notify();
            });
        });
        self.registry.set_task(task);
        cx.notify();
    }
    fn choose(&mut self, reference: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        self.registry.select(reference.clone());
        self.input
            .update(cx, |input, cx| input.set_value(reference, window, cx));
        cx.emit(ImageChanged);
        cx.notify();
    }
    fn local_images(
        &self,
        height: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let query = self.input.read(cx).value().trim().to_ascii_lowercase();
        let rows: Vec<_> = self
            .local
            .images
            .iter()
            .filter(|image| {
                [
                    image.repository.as_deref().unwrap_or(""),
                    image.tag.as_deref().unwrap_or(""),
                    &Self::reference(image),
                ]
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(&query))
            })
            .take(25)
            .collect();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .when_some(self.local.error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.clone()),
                )
            })
            .child(
                v_flex()
                    .id("project-available-images")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .max_h(height)
                    .overflow_y_scroll()
                    .when(rows.is_empty() && self.local.error.is_none(), |view| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("No matching local images. You can enter a full reference."),
                        )
                    })
                    .children(rows.into_iter().map(|image| {
                        let reference = Self::reference(image);
                        self.image_row(
                            format!("project-local-image-{}", image.id),
                            reference.clone(),
                            image.size.clone(),
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.choose(reference.clone(), window, cx),
                        ))
                    })),
            )
    }
    fn reference(image: &Image) -> String {
        match (&image.repository, &image.tag) {
            (Some(repository), Some(tag)) => format!("{repository}:{tag}"),
            (Some(repository), None) => repository.clone(),
            _ => image.id.clone(),
        }
    }
    fn registry_images(
        &self,
        height: gpui_kit::Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex().w_full().min_w_0().gap_1().child(
            v_flex()
                .id("project-registry-images")
                .test_support()
                .w_full()
                .min_w_0()
                .max_h(height)
                .overflow_y_scroll()
                .children(
                    self.registry
                        .results()
                        .iter()
                        .map(|image| self.registry_row(image, cx)),
                ),
        )
    }
    fn result_sources(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let owner = cx.weak_entity();
        gpui_kit::component::tab::TabBar::new("project-image-sources")
            .underline()
            .small()
            .selected_index(self.results_source)
            .children(
                [
                    ("project-image-source-local", "Local images"),
                    (
                        "project-image-source-registry",
                        if self.engine == Engine::Docker {
                            "Docker Hub"
                        } else {
                            "Registries"
                        },
                    ),
                ]
                .map(|(id, label)| {
                    gpui_kit::component::tab::Tab::new()
                        .aria_label(label)
                        .disabled(self.locked)
                        .child(div().id(id).test_support().child(label))
                }),
            )
            .on_click(move |index, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.results_source = *index;
                    cx.notify();
                });
            })
    }
    fn status(&self) -> String {
        if self.search.is_none() || self.source.executor().is_none() {
            return "Registry search is unavailable. Enter a full reference.".into();
        }
        match self.registry.status() {
            SearchStatus::Idle => {
                "Type at least 2 characters to search. Tags can be entered directly.".into()
            }
            SearchStatus::Pending => "Searching repositories…".into(),
            SearchStatus::Complete if self.registry.results().is_empty() => {
                "No matching repositories. Enter a full reference to use another image.".into()
            }
            SearchStatus::Complete => {
                "Repository results; enter a tag or digest directly when needed.".into()
            }
            SearchStatus::Failed(message) => message.clone(),
        }
    }
    fn registry_row(&self, image: &RegistryImage, cx: &mut Context<Self>) -> Button {
        let reference = image.reference.clone();
        let description = if image.official {
            format!("Official · {}", image.description)
        } else {
            image.description.clone()
        };
        self.image_row(
            format!("project-registry-image-{reference}"),
            reference.clone(),
            description,
            cx,
        )
        .on_click(
            cx.listener(move |this, _, window, cx| this.choose(reference.clone(), window, cx)),
        )
    }
    fn image_row(&self, id: String, reference: String, detail: String, cx: &App) -> Button {
        let selected = self.input.read(cx).value().as_ref() == reference;
        Button::new(SharedString::from(id.clone()))
            .ghost()
            .w_full()
            .min_w_0()
            .h_auto()
            .py_1()
            .justify_start()
            .selected(selected)
            .disabled(self.locked)
            .role(gpui_kit::Role::RadioButton)
            .toggled(selected)
            .accessibility_label(reference.clone())
            .tooltip(reference.clone())
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-reference")))
                            .test_support()
                            .w_full()
                            .min_w_0()
                            .text_sm()
                            .text_left()
                            .truncate()
                            .child(reference),
                    )
                    .when(!detail.is_empty(), |view| {
                        view.child(
                            div()
                                .w_full()
                                .min_w_0()
                                .text_xs()
                                .text_left()
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail),
                        )
                    }),
            )
    }
}
impl Render for ImagePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let result_height = (window.viewport_size().height - px(540.))
            .max(px(100.))
            .min(px(180.));
        let choices = [
            (WorkspaceImage::Ubuntu, "Ubuntu 24.04"),
            (WorkspaceImage::Debian, "Debian 12"),
            (WorkspaceImage::Custom(String::new()), "Custom image"),
        ];
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(h_flex().gap_2().children(choices.map(|(image, name)| {
                Button::new(SharedString::from(format!("project-image-{name}")))
                    .ghost()
                    .small()
                    .label(name)
                    .selected(std::mem::discriminant(&self.image) == std::mem::discriminant(&image))
                    .disabled(self.locked)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.image = image.clone();
                        this.refresh(cx);
                        cx.emit(ImageChanged);
                        cx.notify();
                    }))
            })))
            .when(self.is_custom(), |view| {
                view.child(
                    div().id("project-custom-image").test_support().child(
                        Input::new(&self.input)
                            .id("input")
                            .aria_label("Image reference")
                            .disabled(self.locked),
                    ),
                )
                .child(self.result_sources(cx))
                .when(self.results_source == 1, |view| {
                    view.child(
                        div()
                            .id("project-registry-status")
                            .test_support()
                            .role(gpui_kit::Role::Status)
                            .aria_label(self.status())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.status()),
                    )
                })
                .when(self.results_source == 0, |view| {
                    view.child(self.local_images(result_height, cx))
                })
                .when(self.results_source == 1, |view| {
                    view.child(self.registry_images(result_height, cx))
                })
            })
    }
}
