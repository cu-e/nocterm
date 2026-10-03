use crate::{
    runtime::Runtime,
    thread::{AgentThread, Attachment},
};
use base64::Engine as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, ListAlignment, ListState,
    PathPromptOptions, SharedString, Subscription, Task, TestSupportExt as _, WeakEntity, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        text::TextView,
        v_flex,
    },
    div, img, list,
    prelude::*,
    px, rems,
};
use nocterm_ai::{acp, thread::Entry};
use nocterm_ui::{ActiveAi as _, ActiveSettings as _, IconName, SettingsStore};
use nocterm_workspace::{Panel, RightPanel, RightPanelEvent, Workspace};
use std::io::Read as _;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
#[derive(Clone, PartialEq, Eq)]
enum MenuKind {
    Agents,
    Context,
    Config(String),
    Modes,
}
enum CachedImage {
    Loading(Option<Task<()>>),
    Ready(Arc<gpui_kit::Image>),
    Failed,
}
pub(crate) struct AgentPanel {
    image_cache: HashMap<(usize, usize, bool), CachedImage>,
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
    threads: Vec<Entity<AgentThread>>,
    active: Option<usize>,
    history: bool,
    maximized: bool,
    input: Entity<TextareaState>,
    search: Entity<InputState>,
    menu: Option<MenuKind>,
    list: ListState,
    list_count: usize,
    expanded: HashSet<usize>,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
    notify_queued: bool,
}
impl EventEmitter<RightPanelEvent> for AgentPanel {}
impl AgentPanel {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .submit_on_enter(true)
                .placeholder("Ask about your terminals…")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
        let submit = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
        });
        let search_observer = cx.observe(&search, |_, _, cx| cx.notify());
        let settings = cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
            let enabled = cx.ai_enabled();
            if !enabled {
                this.threads.clear();
                this.image_cache.clear();
                this.active = None;
                this.menu = None;
                this.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            let workspace = this.workspace.clone();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.set_right_panel_available(enabled, window, cx)
                });
            });
            cx.notify();
        });
        let workspace_observer = workspace
            .upgrade()
            .map(|workspace| cx.observe(&workspace, |_, _, cx| cx.notify()));
        let runtime_observer = cx.observe(&Runtime::global(cx), |_, _, cx| cx.notify());
        Self {
            image_cache: HashMap::new(),
            focus: cx.focus_handle(),
            workspace,
            threads: Vec::new(),
            active: None,
            history: false,
            maximized: false,
            input,
            search,
            menu: None,
            list: ListState::new(0, ListAlignment::Bottom, px(500.)),
            list_count: 0,
            expanded: HashSet::new(),
            error: None,
            subscriptions: [
                Some(submit),
                Some(search_observer),
                Some(settings),
                Some(runtime_observer),
                workspace_observer,
            ]
            .into_iter()
            .flatten()
            .collect(),
            notify_queued: false,
        }
    }
    fn current(&self) -> Option<Entity<AgentThread>> {
        self.active
            .and_then(|index| self.threads.get(index))
            .cloned()
    }
    fn new_thread(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread_connection(id, false, window, cx);
    }
    fn new_thread_connection(
        &mut self,
        id: String,
        fresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(launch) = nocterm_ai::AgentRegistry::new(&cx.settings().ai)
            .get(&id)
            .cloned()
        else {
            return;
        };
        let active = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).active_terminal(cx));
        let thread = cx.new(|cx| AgentThread::new(id, self.workspace.clone(), cx));
        if let Some(id) = active {
            thread.update(cx, |thread, _| {
                thread.attachments.push(Attachment::Terminal(id))
            });
        }
        let runtime = Runtime::global(cx);
        let registration = runtime.update(cx, |runtime, cx| runtime.register_bridge(&thread, cx));
        match registration {
            Ok(registration) => {
                thread.update(cx, |thread, _| thread.registration = Some(registration))
            }
            Err(error) => thread.update(cx, |thread, cx| thread.fail(&error, cx)),
        };
        self.subscriptions
            .push(cx.observe_in(&thread, window, |this, _, window, cx| {
                let attention = this.threads.iter().any(|thread| {
                    !thread.read(cx).tools.is_empty() || !thread.read(cx).permissions.is_empty()
                });
                let workspace = this.workspace.clone();
                window.defer(cx, move |window, cx| {
                    let _ = workspace.update(cx, |workspace, cx| {
                        workspace.set_right_panel_attention(attention, cx)
                    });
                    if attention {
                        let show = workspace.clone();
                        nocterm_ui::notice::warning_action(
                            window,
                            cx,
                            "agent-approval",
                            "Agent request",
                            "An agent needs permission to continue.",
                            "Review",
                            move |window, cx| {
                                let _ = show.update(cx, |workspace, cx| {
                                    if !workspace.right_panel_is_open() {
                                        workspace.toggle_right_panel(window, cx);
                                    }
                                });
                            },
                        );
                    }
                });
                if this.notify_queued {
                    return;
                }
                this.notify_queued = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(33))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.notify_queued = false;
                        cx.notify();
                    });
                })
                .detach();
            }));
        if thread.read(cx).registration.is_some() {
            runtime.update(cx, |runtime, cx| {
                runtime.connect(thread.clone(), launch, fresh, cx)
            });
        }
        self.threads.push(thread);
        self.active = Some(self.threads.len() - 1);
        self.history = false;
        self.menu = None;
        self.list.reset(0);
        self.list_count = 0;
        self.expanded.clear();
        self.image_cache.clear();
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        if thread.read(cx).generating
            || thread.read(cx).session.is_none()
            || !thread.read(cx).accept_updates
        {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        thread.update(cx, |thread, cx| thread.send(text, cx));
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.menu = None;
        cx.notify();
    }
    fn toggle_menu(&mut self, menu: MenuKind, cx: &mut Context<Self>) {
        self.menu = if self.menu.as_ref() == Some(&menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }
    fn add_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(thread) = self.current() else {
            return;
        };
        if !thread
            .read(cx)
            .info
            .as_ref()
            .is_some_and(|info| info.capabilities.prompt_capabilities.image)
        {
            return;
        }
        let epoch = thread.read(cx).epoch;
        let thread = thread.downgrade();
        let future = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    std::io::Read::take(
                        &mut file,
                        (nocterm_ai::images::MAX_IMAGE_BYTES + 1) as u64,
                    )
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                    nocterm_ai::images::PromptImage::validate(bytes)
                })
                .collect::<Result<Vec<_>, String>>()
        });
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(images) => {
                        let _ = thread.update(cx, |thread, cx| {
                            if thread.epoch != epoch || !cx.ai_enabled() {
                                return;
                            }
                            let mut collection = thread.images.clone();
                            collection.extend(images);
                            match nocterm_ai::images::validate_collection(&collection) {
                                Ok(()) => thread.images = collection,
                                Err(error) => this.error = Some(error),
                            }
                            cx.notify();
                        });
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn pick_images(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| this.add_images(paths, cx));
            }
        })
        .detach();
    }
    fn image_view(&self, key: (usize, usize, bool)) -> gpui_kit::AnyElement {
        match self.image_cache.get(&key) {
            Some(CachedImage::Ready(image)) => img(image.clone())
                .max_w_full()
                .h(rems(10.))
                .into_any_element(),
            Some(CachedImage::Failed) => {
                div().child("Image could not be loaded.").into_any_element()
            }
            _ => div().child("Loading image…").into_any_element(),
        }
    }
    fn cached_image(
        &mut self,
        key: (usize, usize, bool),
        data: impl FnOnce() -> Vec<u8>,
        encoded: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.image_cache.entry(key) {
            entry.insert(CachedImage::Loading(None));
            let bytes = data();
            let future = cx.background_executor().spawn(async move {
                let bytes = if encoded {
                    if bytes.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                        return Err("Image exceeds size limit".to_owned());
                    }
                    base64::engine::general_purpose::STANDARD
                        .decode(bytes)
                        .map_err(|error| error.to_string())?
                } else {
                    bytes
                };
                let image = nocterm_ai::images::PromptImage::validate(bytes)?;
                let thumbnail = image::load_from_memory(&image.data)
                    .map_err(|error| error.to_string())?
                    .thumbnail(640, 640);
                let mut png = std::io::Cursor::new(Vec::new());
                thumbnail
                    .write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(Arc::new(gpui_kit::Image::from_bytes(
                    gpui_kit::ImageFormat::Png,
                    png.into_inner(),
                )))
            });
            let task = cx.spawn(async move |this, cx| {
                let result = future.await;
                let _ = this.update(cx, |this, cx| {
                    if let std::collections::hash_map::Entry::Occupied(mut entry) =
                        this.image_cache.entry(key)
                    {
                        entry.insert(match result {
                            Ok(image) => CachedImage::Ready(image),
                            Err(_) => CachedImage::Failed,
                        });
                        this.list.remeasure();
                        cx.notify();
                    }
                });
            });
            if let Some(CachedImage::Loading(pending)) = self.image_cache.get_mut(&key) {
                *pending = Some(task);
            }
        }
        match self.image_cache.get(&key) {
            Some(CachedImage::Ready(image)) => img(image.clone())
                .max_w_full()
                .h(rems(10.))
                .into_any_element(),
            Some(CachedImage::Failed) => {
                div().child("Image could not be loaded.").into_any_element()
            }
            _ => div().child("Loading image…").into_any_element(),
        }
    }
    fn render_entry(&mut self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let Some(entry) = thread.read(cx).state.entries.get(index) else {
            return div().into_any_element();
        };
        let mut pending_images = Vec::new();
        let expanded = self.expanded.contains(&index);
        let mut row = v_flex()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border);
        match entry {
            Entry::User(parts) => {
                row = row.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("You"),
                );
                for part in parts {
                    match part {
                        acp::ContentBlock::Text(text) => row = row.child(text.text.clone()),
                        acp::ContentBlock::Image(image) => {
                            let key = (image.data.as_ptr() as usize, image.data.len(), true);
                            if !self.image_cache.contains_key(&key) {
                                pending_images.push((key, image.data.as_bytes().to_vec()));
                            }
                            row = row.child(self.image_view(key))
                        }
                        _ => {}
                    }
                }
            }
            Entry::Agent(text) => {
                row = row.child(TextView::markdown(
                    ("agent-message", index),
                    safe_markdown(text),
                ))
            }
            Entry::Thought(text) => {
                row = row.child(
                    Button::new(("thought", index))
                        .ghost()
                        .small()
                        .icon(IconName::Brain)
                        .label(if expanded {
                            "Hide reasoning"
                        } else {
                            "Reasoning"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.insert(index) {
                                this.expanded.remove(&index);
                            }
                            this.list.remeasure_items(index..index + 1);
                            cx.notify();
                        })),
                );
                if expanded {
                    row = row.child(TextView::markdown(
                        ("thought-text", index),
                        safe_markdown(text),
                    ));
                }
            }
            Entry::Tool(call) => {
                row = row.child(
                    Button::new(("tool-call", index))
                        .ghost()
                        .small()
                        .icon(IconName::Wrench)
                        .label(format!("{} · {:?}", call.title, call.status))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.insert(index) {
                                this.expanded.remove(&index);
                            }
                            this.list.remeasure_items(index..index + 1);
                            cx.notify();
                        })),
                );
                if expanded {
                    let text = serde_json::to_string_pretty(&call.content).unwrap_or_default();
                    let mut end = text.len().min(200 * 1024);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    row = row.child(text[..end].to_owned());
                    if end < text.len() {
                        row = row.child("Tool output truncated at 200 KiB.");
                    }
                }
            }
            Entry::Content(acp::ContentBlock::Image(image)) => {
                let key = (image.data.as_ptr() as usize, image.data.len(), true);
                if !self.image_cache.contains_key(&key) {
                    pending_images.push((key, image.data.as_bytes().to_vec()));
                }
                row = row.child(self.image_view(key))
            }
            Entry::Content(_) => row = row.child("Agent supplied additional content."),
        }
        for (key, data) in pending_images {
            self.cached_image(key, || data, true, cx);
        }
        row.into_any_element()
    }
    fn render_menu(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut menu = v_flex()
            .id("agent-picker")
            .test_support()
            .gap_1()
            .p_2()
            .max_h(rems(18.))
            .overflow_y_scroll()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background);
        match self.menu.clone() {
            Some(MenuKind::Agents) => {
                for launch in nocterm_ai::AgentRegistry::new(&cx.settings().ai).iter() {
                    let id = launch.id.clone();
                    menu = menu.child(
                        Button::new(SharedString::from(format!("new-agent-{id}")))
                            .ghost()
                            .label(launch.name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.new_thread(id.clone(), window, cx)
                            })),
                    );
                }
            }
            Some(MenuKind::Context) => {
                if let (Some(thread), Some(workspace)) = (self.current(), self.workspace.upgrade())
                {
                    for entry in workspace.read(cx).terminals(cx) {
                        let attachment = Attachment::Terminal(entry.item);
                        let selected = thread.read(cx).attachments.contains(&attachment);
                        menu = menu.child(
                            Button::new(SharedString::from(format!("attach-{:?}", entry.item)))
                                .ghost()
                                .label(entry.title.clone())
                                .selected(selected)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(thread) = this.current() {
                                        thread.update(cx, |thread, cx| {
                                            thread.attach(attachment.clone(), cx)
                                        });
                                    }
                                })),
                        );
                    }
                    if let Some(directory) = workspace.read(cx).connection_directory() {
                        let summaries = directory.connections(cx);
                        let mut groups = HashSet::new();
                        for summary in summaries {
                            if let Some(group) = &summary.group
                                && groups.insert(group.to_string())
                            {
                                let attachment = Attachment::Group(group.to_string());
                                let selected = thread.read(cx).attachments.contains(&attachment);
                                menu = menu.child(
                                    Button::new(SharedString::from(format!(
                                        "attach-group-{group}"
                                    )))
                                    .ghost()
                                    .label(format!("All in {group}"))
                                    .selected(selected)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(thread) = this.current() {
                                            thread.update(cx, |thread, cx| {
                                                thread.attach(attachment.clone(), cx)
                                            });
                                        }
                                    })),
                                );
                            }
                            let attachment = Attachment::Connection(summary.id.to_string());
                            let selected = thread.read(cx).attachments.contains(&attachment);
                            let id = summary.id.to_string();
                            let open_directory = directory.clone();
                            let open_workspace = self.workspace.clone();
                            menu =
                                menu.child(
                                    h_flex()
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "attach-connection-{id}"
                                            )))
                                            .ghost()
                                            .label(summary.name)
                                            .selected(selected)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(thread) = this.current() {
                                                    thread.update(cx, |thread, cx| {
                                                        thread.attach(attachment.clone(), cx)
                                                    });
                                                }
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "open-connection-{id}"
                                            )))
                                            .ghost()
                                            .small()
                                            .label("Open")
                                            .on_click(move |_, window, cx| {
                                                let _ = open_workspace.update(cx, |_, cx| {
                                                    open_directory.open(&id, window, cx);
                                                });
                                            }),
                                        ),
                                );
                        }
                    }
                }
            }
            Some(MenuKind::Config(id)) => {
                if let Some(thread) = self.current() {
                    let options = thread.read(cx).state.config_options.clone();
                    if let Some(option) = options.iter().find(|option| option.id.0.as_ref() == id) {
                        menu = menu.child(Input::new(&self.search).small());
                        let query =
                            if option.category == Some(acp::SessionConfigOptionCategory::Model) {
                                self.search.read(cx).value().to_lowercase()
                            } else {
                                String::new()
                            };
                        match &option.kind {
                            acp::SessionConfigKind::Select(select) => {
                                let groups = match &select.options {
                                    acp::SessionConfigSelectOptions::Ungrouped(options) => {
                                        vec![(String::new(), options.clone())]
                                    }
                                    acp::SessionConfigSelectOptions::Grouped(groups) => groups
                                        .iter()
                                        .map(|group| (group.name.clone(), group.options.clone()))
                                        .collect(),
                                    _ => Vec::new(),
                                };
                                for (group, mut values) in groups {
                                    if !group.is_empty() {
                                        menu = menu.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(group),
                                        );
                                    }
                                    values.sort_by_key(|value| {
                                        !Runtime::global(cx).read(cx).favorites.contains(
                                            &thread.read(cx).agent_id,
                                            &id,
                                            value.value.0.as_ref(),
                                        )
                                    });
                                    for value in values
                                        .into_iter()
                                        .filter(|value| value.name.to_lowercase().contains(&query))
                                    {
                                        let favorite =
                                            Runtime::global(cx).read(cx).favorites.contains(
                                                &thread.read(cx).agent_id,
                                                &id,
                                                value.value.0.as_ref(),
                                            );
                                        let value_id = value.value.clone();
                                        let config = option.id.clone();
                                        let star_value = value.value.to_string();
                                        let agent = thread.read(cx).agent_id.clone();
                                        let star_id = id.clone();
                                        menu=menu.child(h_flex().child(Button::new(SharedString::from(format!("config-{id}-{value_id}"))).ghost().label(value.name).selected(value.value==select.current_value).on_click(cx.listener(move|this,_,_,cx|{if let Some(thread)=this.current(){thread.update(cx,|thread,cx|thread.set_config(config.clone(),acp::SessionConfigOptionValue::ValueId{value:value_id.clone()},cx));}this.menu=None;cx.notify();})))
                                .when(option.category==Some(acp::SessionConfigOptionCategory::Model),|row|row.child(Button::new(SharedString::from(format!("favorite-{star_id}-{star_value}"))).ghost().small().icon(if favorite{IconName::StarFill}else{IconName::Star}).tooltip("Favorite model").on_click(move|_,_,cx|Runtime::global(cx).update(cx,|runtime,cx|runtime.toggle_favorite(&agent,&star_id,&star_value,cx))))));
                                    }
                                }
                            }
                            acp::SessionConfigKind::Boolean(boolean) => {
                                let config = option.id.clone();
                                let value = !boolean.current_value;
                                menu = menu.child(
                                    Button::new("config-boolean")
                                        .ghost()
                                        .label(if value { "Enable" } else { "Disable" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(thread) = this.current() {
                                                thread.update(cx, |thread, cx| {
                                                    thread.set_config(
                                                        config.clone(),
                                                        acp::SessionConfigOptionValue::Boolean {
                                                            value,
                                                        },
                                                        cx,
                                                    )
                                                });
                                            }
                                            this.menu = None;
                                            cx.notify();
                                        })),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(MenuKind::Modes) => {
                if let Some(thread) = self.current()
                    && let Some(modes) = &thread.read(cx).state.modes
                {
                    for mode in &modes.available_modes {
                        let id = mode.id.clone();
                        menu = menu.child(
                            Button::new(SharedString::from(format!("mode-{id}")))
                                .ghost()
                                .label(mode.name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(thread) = this.current() {
                                        thread.update(cx, |thread, cx| {
                                            thread.set_mode(id.clone(), cx)
                                        });
                                    }
                                    this.menu = None;
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
            None => {}
        }
        menu.into_any_element()
    }
}
impl Focusable for AgentPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Panel for AgentPanel {
    fn title(&self, _: &App) -> SharedString {
        "AI Agents".into()
    }
    fn icon(&self, _: &App) -> IconName {
        IconName::Bot
    }
}
impl RightPanel for AgentPanel {
    fn set_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        self.maximized = maximized;
        cx.notify();
    }
}
impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        let mut body =
            v_flex()
                .id("agent-panel")
                .test_support()
                .key_context("AgentPanel")
                .track_focus(&self.focus)
                .size_full()
                .min_w_0()
                .bg(cx.theme().background)
                .on_action(cx.listener(|this, _: &crate::NewThread, _, cx| {
                    this.toggle_menu(MenuKind::Agents, cx)
                }))
                .on_action(cx.listener(|this, _: &crate::ShowHistory, _, cx| {
                    this.history = !this.history;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &crate::StopGeneration, _, cx| {
                    if let Some(thread) = this.current() {
                        thread.update(cx, |thread, cx| thread.stop(cx));
                    }
                }))
                .on_action(cx.listener(|this, _: &crate::ToggleModelPicker, _, cx| {
                    if let Some(thread) = this.current()
                        && let Some(option) =
                            thread.read(cx).state.config_options.iter().find(|option| {
                                option.category == Some(acp::SessionConfigOptionCategory::Model)
                            })
                    {
                        this.toggle_menu(MenuKind::Config(option.id.to_string()), cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &crate::AttachImage, _, cx| this.pick_images(cx)))
                .child(
                    h_flex()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(IconName::Bot)
                        .child("AI Agents")
                        .child(div().flex_1())
                        .child(
                            Button::new("agent-history")
                                .ghost()
                                .small()
                                .icon(IconName::Clock)
                                .tooltip("Chat history")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.history = !this.history;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("agent-new-thread")
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .tooltip("New chat")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_menu(MenuKind::Agents, cx)
                                })),
                        )
                        .child(
                            Button::new("agent-maximize")
                                .ghost()
                                .small()
                                .icon(if self.maximized {
                                    IconName::Minimize2
                                } else {
                                    IconName::Maximize2
                                })
                                .tooltip(if self.maximized {
                                    "Restore panel"
                                } else {
                                    "Expand chat"
                                })
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(RightPanelEvent::ToggleMaximized)
                                })),
                        ),
                );
        if self.history || current.is_none() {
            let mut history = v_flex()
                .id("agent-history-list")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .gap_2();
            if self.threads.is_empty() {
                history =
                    history
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child("History is empty"),
                        )
                        .child(
                            Button::new("agent-empty-new")
                                .primary()
                                .label("New chat")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_menu(MenuKind::Agents, cx)
                                })),
                        );
            }
            for (index, thread) in self.threads.iter().enumerate() {
                let title = thread.read(cx).state.title.clone().unwrap_or_else(|| {
                    thread
                        .read(cx)
                        .state
                        .entries
                        .iter()
                        .find_map(|entry| {
                            if let Entry::User(parts) = entry {
                                parts.iter().find_map(|part| {
                                    if let acp::ContentBlock::Text(text) = part {
                                        Some(text.text.chars().take(36).collect::<String>())
                                    } else {
                                        None
                                    }
                                })
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| format!("{} chat", thread.read(cx).agent_id))
                });
                history = history.child(
                    h_flex()
                        .child(
                            Button::new(("history-thread", index))
                                .ghost()
                                .label(title)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.active = Some(index);
                                    this.history = false;
                                    this.list.reset(0);
                                    this.list_count = 0;
                                    this.expanded.clear();
                                    this.image_cache.clear();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("delete-thread", index))
                                .ghost()
                                .small()
                                .icon(IconName::Trash)
                                .tooltip("Delete chat")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if index < this.threads.len() {
                                        this.threads.remove(index);
                                        this.active = if this.threads.is_empty() {
                                            None
                                        } else {
                                            Some(index.min(this.threads.len() - 1))
                                        };
                                        this.list.reset(0);
                                        this.list_count = 0;
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            body = body.child(history);
        } else if let Some(thread) = current {
            let model = &thread.read(cx).state;
            let count = model.entries.len();
            let state = nocterm_ai::thread::ThreadState {
                config_options: model.config_options.clone(),
                modes: model.modes.clone(),
                current_mode: model.current_mode.clone(),
                plan: model.plan.clone(),
                usage: model.usage.clone(),
                ..Default::default()
            };
            if count != self.list_count {
                if count > self.list_count {
                    self.list
                        .splice(self.list_count..self.list_count, count - self.list_count);
                } else {
                    self.list.reset(count);
                }
                self.list_count = count;
            } else if count > 0 {
                self.list.remeasure_items(0..count);
            }
            body = body.child(
                list(
                    self.list.clone(),
                    cx.processor(|this, index, _, cx| this.render_entry(index, cx)),
                )
                .flex_1()
                .min_h_0(),
            );
            if let Some(plan) = &state.plan {
                body = body.child(
                    v_flex()
                        .p_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .children(plan.entries.iter().map(|entry| {
                            div()
                                .text_sm()
                                .child(format!("{:?} · {}", entry.status, entry.content))
                        })),
                );
            }
            for (index, permission) in thread.read(cx).permissions.iter().enumerate() {
                let mut card = v_flex()
                    .p_3()
                    .gap_2()
                    .border_1()
                    .border_color(cx.theme().warning)
                    .child(
                        permission
                            .request
                            .tool_call
                            .fields
                            .title
                            .clone()
                            .unwrap_or_else(|| "Agent permission request".into()),
                    );
                for option in &permission.request.options {
                    let id = option.option_id.clone();
                    card = card.child(
                        Button::new(SharedString::from(format!("permission-{index}-{id}")))
                            .label(option.name.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| {
                                        thread.choose_permission(index, Some(id.clone()), cx)
                                    });
                                }
                            })),
                    );
                }
                body = body.child(card);
            }
            let targets = thread.update(cx, |thread, cx| thread.resolved(cx));
            for (index, call) in thread.read(cx).tools.iter().enumerate() {
                let target = targets
                    .iter()
                    .find(|(id, _, _)| Some(id.as_str()) == call.call.terminal_id())
                    .map(|(_, entry, descriptor)| {
                        format!(
                            "{}{}",
                            entry.title,
                            descriptor
                                .connection
                                .as_ref()
                                .map(|connection| format!(
                                    " · {}@{}:{}",
                                    connection.user, connection.host, connection.port
                                ))
                                .unwrap_or_default()
                        )
                    })
                    .unwrap_or_else(|| "Unavailable terminal".into());
                let exact = match &call.call {
                    nocterm_ai::TerminalCall::ReadTerminal(request) => {
                        format!("Read {target} ({} lines)", request.lines.unwrap_or(200))
                    }
                    nocterm_ai::TerminalCall::SendInput(request) => format!(
                        "{}: {:?}{}",
                        target,
                        request.text,
                        if request.press_enter { " + Enter" } else { "" }
                    ),
                    nocterm_ai::TerminalCall::RunCommand(request) => {
                        format!("{target}: {}", request.command)
                    }
                    _ => String::new(),
                };
                body = body.child(
                    v_flex()
                        .p_3()
                        .gap_2()
                        .border_1()
                        .border_color(cx.theme().warning)
                        .child("Terminal access request")
                        .child(exact)
                        .child(
                            h_flex().gap_2().children(
                                [
                                    (false, false, "Deny"),
                                    (true, false, "Allow once"),
                                    (true, true, "Allow for this terminal"),
                                ]
                                .into_iter()
                                .map(|(allow, grant, label)| {
                                    Button::new((
                                        SharedString::from(format!("tool-{label}")),
                                        index,
                                    ))
                                    .label(label)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Some(thread) = this.current() {
                                                thread.update(cx, |thread, cx| {
                                                    thread.approve_tool(index, allow, grant, cx)
                                                });
                                            }
                                        },
                                    ))
                                }),
                            ),
                        ),
                );
            }
            let mut composer = v_flex()
                .p_3()
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().border);
            let mut controls = h_flex().gap_1().flex_wrap();
            for option in &state.config_options {
                let id = option.id.to_string();
                controls = controls.child(
                    Button::new(SharedString::from(format!("config-picker-{id}")))
                        .ghost()
                        .small()
                        .label(config_label(option))
                        .disabled(thread.read(cx).generating)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_menu(MenuKind::Config(id.clone()), cx)
                        })),
                );
            }
            if !state.config_overrides_modes() && state.modes.is_some() {
                controls = controls.child(
                    Button::new("mode-picker")
                        .ghost()
                        .small()
                        .label("Mode")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.toggle_menu(MenuKind::Modes, cx)),
                        ),
                );
            }
            composer = composer.child(controls);
            if let Some(usage) = &state.usage {
                composer = composer.child(
                    div()
                        .text_sm()
                        .text_color(
                            if usage.size > 0 && usage.used as f64 / usage.size as f64 >= 0.85 {
                                cx.theme().warning
                            } else {
                                cx.theme().muted_foreground
                            },
                        )
                        .child(format!(
                            "Context: {} / {} tokens{}",
                            usage.used,
                            usage.size,
                            usage
                                .cost
                                .as_ref()
                                .map(|cost| format!(" · {:.4} {}", cost.amount, cost.currency))
                                .unwrap_or_default()
                        )),
                );
            }
            let summaries = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).connection_directory())
                .map(|directory| directory.connections(cx))
                .unwrap_or_default();
            let descriptions = thread.update(cx, |thread, cx| thread.resolved(cx));
            composer = composer.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} attached terminals · sent metadata {} B · terminal output {} B",
                        descriptions.len(),
                        thread.read(cx).context_bytes,
                        thread.read(cx).tool_bytes
                    )),
            );
            for attachment in &thread.read(cx).attachments {
                let value = attachment.clone();
                let label = match attachment {
                    Attachment::Terminal(id) => descriptions
                        .iter()
                        .find(|(_, entry, _)| entry.item == *id)
                        .map(|(_, entry, _)| entry.title.to_string())
                        .unwrap_or_else(|| "Closed terminal".into()),
                    Attachment::Connection(id) => summaries
                        .iter()
                        .find(|summary| summary.id.as_ref() == id)
                        .map(|summary| summary.name.to_string())
                        .unwrap_or_else(|| "Unavailable connection".into()),
                    Attachment::Group(group) => format!("Group {group}"),
                };
                composer = composer.child(
                    Button::new(SharedString::from(format!("chip-{label}")))
                        .ghost()
                        .small()
                        .label(format!("{label} ×"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(thread) = this.current() {
                                thread.update(cx, |thread, cx| thread.attach(value.clone(), cx));
                            }
                        })),
                );
            }
            let previews = thread
                .read(cx)
                .images
                .iter()
                .map(|image| {
                    let key = (image.data.as_ptr() as usize, image.data.len(), false);
                    (
                        key,
                        (!self.image_cache.contains_key(&key)).then(|| image.data.clone()),
                    )
                })
                .collect::<Vec<_>>();
            for (index, (key, data)) in previews.into_iter().enumerate() {
                let thumbnail = self.cached_image(key, || data.unwrap_or_default(), false, cx);
                composer = composer.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(rems(3.)).h(rems(3.)).child(thumbnail))
                        .child(
                            Button::new(("remove-image", index))
                                .ghost()
                                .small()
                                .icon(IconName::X)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(thread) = this.current() {
                                        thread.update(cx, |thread, cx| {
                                            if index < thread.images.len() {
                                                thread.images.remove(index);
                                            }
                                            cx.notify();
                                        });
                                    }
                                })),
                        ),
                );
            }
            let weak = cx.weak_entity();
            composer = composer.child(Textarea::new(&self.input).on_paste(
                move |clipboard, _, cx| {
                    let paths = clipboard
                        .entries()
                        .iter()
                        .filter_map(|entry| {
                            if let gpui_kit::ClipboardEntry::ExternalPaths(paths) = entry {
                                Some(paths.paths().to_vec())
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .collect::<Vec<_>>();
                    if !paths.is_empty() {
                        let _ = weak.update(cx, |panel, cx| panel.add_images(paths, cx));
                        return true;
                    }
                    if let Some(image) = clipboard.entries().iter().find_map(|entry| {
                        if let gpui_kit::ClipboardEntry::Image(image) = entry {
                            Some(image.clone())
                        } else {
                            None
                        }
                    }) {
                        let _ =
                            weak.update(cx, |panel, cx| {
                                if let Some(thread) = panel.current()
                                    && thread.read(cx).info.as_ref().is_some_and(|info| {
                                        info.capabilities.prompt_capabilities.image
                                    })
                                {
                                    match nocterm_ai::images::PromptImage::validate(image.bytes) {
                                        Ok(image) => {
                                            let mut images = thread.read(cx).images.clone();
                                            images.push(image);
                                            match nocterm_ai::images::validate_collection(&images) {
                                                Ok(()) => thread.update(cx, |thread, cx| {
                                                    thread.images = images;
                                                    cx.notify();
                                                }),
                                                Err(error) => panel.error = Some(error),
                                            }
                                        }
                                        Err(error) => panel.error = Some(error),
                                    }
                                    cx.notify();
                                }
                            });
                        return true;
                    }
                    false
                },
            ));
            composer =
                composer.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("agent-attach-context")
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .tooltip("Attach terminal or connection context")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_menu(MenuKind::Context, cx)
                                })),
                        )
                        .child(
                            Button::new("agent-attach-image")
                                .ghost()
                                .small()
                                .icon(IconName::ImagePlus)
                                .tooltip("Attach image")
                                .disabled(!thread.read(cx).info.as_ref().is_some_and(|info| {
                                    info.capabilities.prompt_capabilities.image
                                }))
                                .on_click(cx.listener(|this, _, _, cx| this.pick_images(cx))),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("agent-send")
                                .primary()
                                .small()
                                .icon(if thread.read(cx).generating {
                                    IconName::CircleStop
                                } else {
                                    IconName::Send
                                })
                                .label(if thread.read(cx).generating {
                                    "Stop"
                                } else {
                                    "Send"
                                })
                                .disabled(thread.read(cx).session.is_none())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this
                                        .current()
                                        .is_some_and(|thread| thread.read(cx).generating)
                                    {
                                        if let Some(thread) = this.current() {
                                            thread.update(cx, |thread, cx| thread.stop(cx));
                                        }
                                    } else {
                                        this.send(window, cx);
                                    }
                                })),
                        ),
                );
            composer = composer.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(thread.read(cx).status.clone()),
            );
            if !thread.read(cx).accept_updates && !thread.read(cx).generating {
                composer =
                    composer.child(Button::new("agent-restart").label("Restart chat").on_click(
                        cx.listener(|this, _, window, cx| {
                            if let Some(old) = this.current() {
                                let id = old.read(cx).agent_id.clone();
                                let state = old.read(cx).state.clone();
                                let attachments = old.read(cx).attachments.clone();
                                this.new_thread_connection(id, true, window, cx);
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| {
                                        thread.state = state;
                                        thread.attachments = attachments;
                                        cx.notify();
                                    });
                                }
                            }
                        }),
                    ));
            }
            if let Some(info) = &thread.read(cx).info {
                for method in &info.auth_methods {
                    let id = method.id().clone();
                    composer = composer.child(
                        Button::new(SharedString::from(format!("authenticate-{id}")))
                            .ghost()
                            .small()
                            .label(format!("Sign in: {}", method.name()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| {
                                        if let Some(commands) = thread.commands.clone() {
                                            let future = cx
                                                .background_executor()
                                                .spawn(commands.authenticate(id.clone()));
                                            cx.spawn(async move |this, cx| {
                                                let result = future.await;
                                                let _ = this.update(cx, |this, cx| {
                                                    this.status = match result {
                                                        Ok(()) => {
                                                            "Signed in. Restart chat to continue."
                                                                .into()
                                                        }
                                                        Err(error) => nocterm_ai::redact::redact(
                                                            &error.to_string(),
                                                        ),
                                                    };
                                                    cx.notify();
                                                });
                                            })
                                            .detach();
                                        }
                                    });
                                }
                            })),
                    );
                }
            }
            body = body.child(composer);
        }
        if self.menu.is_some() {
            body = body.child(self.render_menu(cx));
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if let Some(error) = &Runtime::global(cx).read(cx).favorites_error {
            body = body.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(format!("Favorites were not saved: {error}")),
            );
        }
        let _ = window;
        body
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// Inline Markdown image URLs never trigger a resource request. Images travel
// through explicit ACP image blocks and the same bounded validation as attachments.
fn safe_markdown(text: &str) -> String {
    text.replace("![", "[").replace("<img", "&lt;img")
}
fn config_label(option: &acp::SessionConfigOption) -> String {
    let selected = match &option.kind {
        acp::SessionConfigKind::Select(select) => {
            let options = match &select.options {
                acp::SessionConfigSelectOptions::Ungrouped(values) => {
                    values.iter().collect::<Vec<_>>()
                }
                acp::SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| group.options.iter())
                    .collect(),
                _ => Vec::new(),
            };
            options
                .iter()
                .find(|value| value.value == select.current_value)
                .map(|value| value.name.clone())
                .unwrap_or_else(|| select.current_value.to_string())
        }
        acp::SessionConfigKind::Boolean(value) => {
            if value.current_value {
                "On".into()
            } else {
                "Off".into()
            }
        }
        _ => String::new(),
    };
    format!("{}: {selected}", option.name)
}
