//! Creation modes retain the original host capability.
mod container_picker;
mod container_row;
use container_picker::{ContainerPicker, ContainerPickerEvent};
mod image_picker;
mod registry_search;
use super::navigation::WizardStep;
use crate::Installed;
use crate::languages::{LanguageChooser, LanguagesChanged};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Subscription, Window,
    component::input::{InputEvent, InputState},
};
use image_picker::{ImageChanged, ImagePicker};
use nocterm_containers::Engine;
use nocterm_explorer::directory::Directory;
use nocterm_project_model::{HostLocator, ProjectId, RootPath};
use nocterm_project_providers::{GitRepository, ProjectSetupError, ProjectSetupRequest, SetupMode};
use nocterm_ui::directory::{DirectoryPicker, DirectoryPickerEvent};
use nocterm_workspace_ports::ProjectCreationSource;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreationMode {
    Direct,
    Existing,
    New,
}
struct ContainerChoices {
    mode: CreationMode,
    engine: Engine,
}
struct CreationInputs {
    path: Entity<InputState>,
    container_path: Entity<InputState>,
    git: Entity<InputState>,
}
struct CreationControls {
    inputs: CreationInputs,
    images: Entity<ImagePicker>,
    languages: Entity<LanguageChooser>,
}
impl CreationControls {
    fn new(
        source: &ProjectCreationSource,
        installed: &Installed,
        container_default: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let path = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("/srv/my-project");
            input.set_value(
                source.cwd().and_then(|cwd| cwd.to_str()).unwrap_or(""),
                window,
                cx,
            );
            input
        });
        let container_path =
            cx.new(|cx| InputState::new(window, cx).placeholder(container_default.to_owned()));
        let git = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://github.com/team/project.git")
        });
        let images = cx.new(|cx| {
            ImagePicker::new(
                source.clone(),
                installed.image_search().cloned(),
                window,
                cx,
            )
        });
        let languages = cx.new(|cx| LanguageChooser::new(window, cx));
        Self {
            inputs: CreationInputs {
                path,
                container_path,
                git,
            },
            images,
            languages,
        }
    }
}
pub(super) struct WorkspaceDraftChanged;

pub(super) struct WorkspaceOptions {
    source: ProjectCreationSource,
    choices: ContainerChoices,
    controls: CreationControls,
    picker: Option<Entity<DirectoryPicker>>,
    reader: Option<std::sync::Arc<dyn nocterm_explorer::directory::DirectoryRead>>,
    containers: Entity<ContainerPicker>,
    container_default: String,
    locked: bool,
    step: WizardStep,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<WorkspaceDraftChanged> for WorkspaceOptions {}
impl WorkspaceOptions {
    pub(super) fn new(
        source: ProjectCreationSource,
        installed: Installed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let container_default = installed
            .settings()
            .setting::<nocterm_project_providers::WorkspacesSettings>(cx)
            .container_directory
            .clone();
        let containers = cx.new(|cx| {
            ContainerPicker::new(source.clone(), installed.preparation().cloned(), window, cx)
        });
        let controls = CreationControls::new(&source, &installed, &container_default, window, cx);
        let (reader, picker) = Self::directory_controls(&source, &installed, window, cx);
        let mut subscriptions = vec![];
        for input in [
            &controls.inputs.path,
            &controls.inputs.container_path,
            &controls.inputs.git,
        ] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.draft_changed(cx);
                }
            }));
        }
        subscriptions.push(cx.subscribe(
            &containers,
            |this, containers, event: &ContainerPickerEvent, cx| match event {
                ContainerPickerEvent::Changed => this.draft_changed(cx),
                ContainerPickerEvent::Inventory => this.controls.images.update(cx, |images, cx| {
                    images.inventory(containers.read(cx).inventory());
                    cx.notify()
                }),
            },
        ));
        subscriptions.push(
            cx.subscribe(&controls.images, |this, _, _: &ImageChanged, cx| {
                this.controls
                    .languages
                    .update(cx, |languages, cx| languages.configure(this.locked, cx));
                this.draft_changed(cx);
            }),
        );
        subscriptions.push(
            cx.subscribe(&controls.languages, |this, _, _: &LanguagesChanged, cx| {
                this.draft_changed(cx)
            }),
        );
        if let Some(picker) = &picker {
            subscriptions.push(cx.subscribe(
                picker,
                |this, _, event: &DirectoryPickerEvent, cx| {
                    if matches!(event, DirectoryPickerEvent::Changed) {
                        this.draft_changed(cx);
                    }
                },
            ));
        }
        Self {
            source,
            choices: ContainerChoices {
                mode: CreationMode::Direct,
                engine: Engine::Docker,
            },
            controls,
            picker,
            reader,
            containers,
            container_default,
            locked: false,
            step: WizardStep::Connection,
            _subscriptions: subscriptions,
        }
    }
    fn directory_controls(
        source: &ProjectCreationSource,
        installed: &Installed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (
        Option<std::sync::Arc<dyn nocterm_explorer::directory::DirectoryRead>>,
        Option<Entity<DirectoryPicker>>,
    ) {
        let reader = installed
            .directories()
            .and_then(|readers| match source.parent().host() {
                HostLocator::Local => Some(readers.local()),
                _ => Some(readers.remote(source.filesystem()?)),
            });
        let picker = reader.clone().map(|reader| {
            let initial = source.cwd().and_then(|path| match source.parent().host() {
                HostLocator::Local => Some(Directory::Local(path.to_owned())),
                _ => path.to_str().map(|path| Directory::Remote(path.to_owned())),
            });
            cx.new(|cx| {
                let mut picker = DirectoryPicker::new(reader, initial, window, cx);
                let root = if matches!(source.parent().host(), HostLocator::Local) {
                    Directory::Local("/".into())
                } else {
                    Directory::Remote("/".into())
                };
                picker.set_fallback(root, cx);
                picker
            })
        });
        (reader, picker)
    }
    pub(super) fn request(
        &self,
        project: ProjectId,
        cx: &App,
    ) -> Result<ProjectSetupRequest, ProjectSetupError> {
        let root = self.root(cx)?;
        let mode =
            match self.choices.mode {
                CreationMode::Direct => SetupMode::Direct {
                    root: root
                        .ok_or(ProjectSetupError::InvalidInput("enter a project directory"))?,
                },
                CreationMode::Existing => {
                    let (engine, container) = self.containers.read(cx).selected().ok_or(
                        ProjectSetupError::InvalidInput("choose an existing container"),
                    )?;
                    SetupMode::Attach {
                        engine,
                        container,
                        root,
                    }
                }
                CreationMode::New => {
                    let image = self.controls.images.read(cx).image(cx);
                    let languages = self.controls.languages.read(cx).selections(cx)?;
                    SetupMode::Create {
                        engine: self.choices.engine,
                        image,
                        languages,
                        root,
                    }
                }
            };
        let repository = self.controls.inputs.git.read(cx).value().to_string();
        let request = ProjectSetupRequest {
            project,
            host: self.source.parent().host().clone(),
            mode,
            repository: if repository.is_empty() {
                None
            } else {
                Some(GitRepository::parse(repository)?)
            },
        };
        request.validate()?;
        Ok(request)
    }
    pub(super) fn new_container(&self) -> bool {
        self.choices.mode == CreationMode::New
    }
    pub(super) fn show_step(&mut self, step: WizardStep, cx: &mut Context<Self>) {
        self.step = step;
        cx.notify();
    }
    fn path(&self, cx: &App) -> String {
        if self.choices.mode == CreationMode::Direct {
            self.picker
                .as_ref()
                .map(|picker| picker.read(cx).draft(cx))
                .unwrap_or_else(|| self.controls.inputs.path.read(cx).value().to_string())
        } else {
            self.controls
                .inputs
                .container_path
                .read(cx)
                .value()
                .to_string()
        }
    }
    fn root(&self, cx: &App) -> Result<Option<RootPath>, ProjectSetupError> {
        let path = self.path(cx);
        if path.is_empty() && self.choices.mode != CreationMode::Direct {
            Ok(Some(RootPath::parse(self.container_default.clone())?))
        } else if path.is_empty() {
            Ok(None)
        } else {
            Ok(Some(RootPath::parse(path)?))
        }
    }
    pub(super) fn focus(&self, step: WizardStep, cx: &App) -> gpui_kit::FocusHandle {
        use gpui_kit::Focusable as _;
        match step {
            WizardStep::Connection if self.choices.mode == CreationMode::Existing => {
                self.containers.read(cx).focus(cx)
            }
            WizardStep::Connection => self.controls.images.read(cx).focus(cx),
            WizardStep::Tools => self.controls.languages.read(cx).validation_focus(cx),
            WizardStep::Project if self.choices.mode != CreationMode::Direct => {
                self.controls.inputs.container_path.focus_handle(cx)
            }
            WizardStep::Project => self
                .picker
                .as_ref()
                .map(|picker| picker.focus_handle(cx))
                .unwrap_or_else(|| self.controls.inputs.path.focus_handle(cx)),
        }
    }
    pub(super) fn validate(
        &self,
        step: WizardStep,
        cx: &App,
    ) -> Result<(), (ProjectSetupError, gpui_kit::FocusHandle)> {
        use gpui_kit::Focusable as _;
        let validation = match step {
            WizardStep::Connection
                if self.choices.mode == CreationMode::Existing
                    && self.containers.read(cx).selected().is_none() =>
            {
                Err(ProjectSetupError::InvalidInput(
                    "choose an existing container",
                ))
            }
            WizardStep::Connection if self.choices.mode == CreationMode::New => {
                self.controls.images.read(cx).image(cx).validate()
            }
            WizardStep::Project => self.root(cx).and_then(|root| {
                if root.is_none() && self.choices.mode == CreationMode::Direct {
                    Err(ProjectSetupError::InvalidInput("enter a project directory"))
                } else {
                    Ok(())
                }
            }),
            WizardStep::Tools if self.new_container() => {
                self.controls.languages.read(cx).selections(cx).map(|_| ())
            }
            _ => Ok(()),
        };
        validation.map_err(|error| (error, self.focus(step, cx)))?;
        if step == WizardStep::Project {
            let repository = self.controls.inputs.git.read(cx).value().to_string();
            if !repository.is_empty() {
                GitRepository::parse(repository)
                    .map_err(|error| (error, self.controls.inputs.git.focus_handle(cx)))?;
            }
        }
        Ok(())
    }
    fn select_mode(&mut self, mode: CreationMode, _window: &mut Window, cx: &mut Context<Self>) {
        if self.locked || self.choices.mode == mode {
            return;
        }
        self.choices.mode = mode;
        self.configure_children(cx);
        self.draft_changed(cx);
    }
    fn draft_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(WorkspaceDraftChanged);
        cx.notify();
    }
    pub(super) fn directory_check(
        &self,
        cx: &App,
    ) -> Option<(
        std::sync::Arc<dyn nocterm_explorer::directory::DirectoryRead>,
        Directory,
    )> {
        if self.choices.mode != CreationMode::Direct
            || !self.controls.inputs.git.read(cx).value().is_empty()
        {
            return None;
        }
        let reader = self.reader.clone()?;
        let draft = self.picker.as_ref()?.read(cx).draft(cx);
        let directory = if matches!(self.source.parent().host(), HostLocator::Local) {
            Directory::Local(draft.into())
        } else {
            Directory::Remote(draft)
        };
        Some((reader, directory))
    }
    pub(super) fn lock(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.locked = locked;
        self.containers
            .update(cx, |containers, cx| containers.lock(locked, cx));
        self.configure_children(cx);
        cx.notify();
    }
    fn configure_children(&self, cx: &mut Context<Self>) {
        self.controls.images.update(cx, |images, cx| {
            images.configure(
                self.choices.engine,
                self.choices.mode == CreationMode::New,
                self.locked,
                cx,
            );
            images.inventory(self.containers.read(cx).inventory());
            cx.notify();
        });
        self.controls
            .languages
            .update(cx, |languages, cx| languages.configure(self.locked, cx));
    }
}

// Familiar native controls keep the three creation paths legible.
use gpui_kit::{
    IntoElement, Render, SharedString,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
};
impl WorkspaceOptions {
    fn modes(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let choices = [
            (
                CreationMode::Direct,
                "project-mode-direct",
                "Direct connection",
            ),
            (
                CreationMode::Existing,
                "project-mode-existing",
                "Existing container",
            ),
            (CreationMode::New, "project-mode-new", "New container"),
        ];
        h_flex().gap_1().children(choices.map(|(mode, id, label)| {
            Button::new(id)
                .ghost()
                .small()
                .label(label)
                .selected(self.choices.mode == mode)
                .role(gpui_kit::Role::RadioButton)
                .toggled(self.choices.mode == mode)
                .accessibility_label(label)
                .disabled(self.locked)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.select_mode(mode, window, cx)),
                )
        }))
    }

    fn engines(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        h_flex().gap_2().children(
            [(Engine::Docker, "Docker"), (Engine::Podman, "Podman")].map(|(engine, name)| {
                Button::new(SharedString::from(format!("project-engine-{name}")))
                    .ghost()
                    .small()
                    .label(name)
                    .selected(self.choices.engine == engine)
                    .disabled(self.locked)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.choices.engine != engine {
                            this.choices.engine = engine;
                            this.configure_children(cx);
                            this.draft_changed(cx);
                        }
                    }))
            }),
        )
    }
}

impl Render for WorkspaceOptions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_3()
            .when(self.step == WizardStep::Connection, |view| {
                view.child(self.modes(cx))
                    .when(self.choices.mode == CreationMode::Direct, |view| view.child(
                        div().text_sm().text_color(cx.theme().muted_foreground)
                            .child("Use a folder on this connection. Choose its directory in the next step.")))
                    .when(self.choices.mode == CreationMode::Existing, |view| view.child(self.containers.clone()))
                    .when(self.choices.mode == CreationMode::New, |view| view.child(self.engines(cx)).child(self.controls.images.clone()))
            })
            .when(self.step == WizardStep::Tools, |view| view
                .child(div().id("project-tool-image").test_support().w_full().min_w_0().truncate().text_sm().text_color(cx.theme().muted_foreground)
                    .aria_label(format!("Install tools in {}", self.controls.images.read(cx).image(cx).reference()))
                    .child(format!("Install tools in {}", self.controls.images.read(cx).image(cx).reference())))
                .child(self.controls.languages.clone()))
            .when(self.step == WizardStep::Project, |view| view
                .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child(
                        if self.choices.mode == CreationMode::Direct {
                            "Project directory"
                        } else {
                            "Project directory (optional)"
                        },
                    ))
                    .child(match (&self.picker, self.choices.mode) {
                        (Some(picker), CreationMode::Direct) if self.locked => div()
                            .id("project-path")
                            .test_support()
                            .text_sm()
                            .child(picker.read(cx).draft(cx))
                            .into_any_element(),
                        (Some(picker), CreationMode::Direct) => div()
                            .id("project-path")
                            .test_support()
                            .child(picker.clone())
                            .into_any_element(),
                        _ => div()
                            .id("project-path")
                            .test_support()
                            .child(
                                Input::new(if self.choices.mode == CreationMode::Direct {
                                    &self.controls.inputs.path
                                } else {
                                    &self.controls.inputs.container_path
                                })
                                .disabled(self.locked),
                            )
                            .into_any_element(),
                    }),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child("Git repository (optional)"),
                    )
                    .child(
                        div()
                            .id("project-git")
                            .test_support()
                            .child(Input::new(&self.controls.inputs.git).disabled(self.locked)),
                    ),
            )
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                    if self.choices.mode == CreationMode::Direct {
                        "An optional repository is cloned when you create the workspace."
                    } else { "Leave the directory empty to use the default shown above. Cloning starts when you create the workspace." }
                )))
    }
}
