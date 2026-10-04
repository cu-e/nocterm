//! User-initiated Zed registry search and per-extension install feedback.
use gpui_kit::{
    App, Context, Entity, SharedString, Subscription, Task, Window,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    prelude::*,
};
use nocterm_themes::{ExtensionInfo, ThemeRegistry, install_archive};
use nocterm_ui::{ActiveThemes as _, Themes, form, reload_themes};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub(super) struct ThemeBrowser {
    registry: Option<Arc<dyn ThemeRegistry>>,
    input: Entity<InputState>,
    results: Vec<ExtensionInfo>,
    searched: bool,
    busy: bool,
    error: Option<SharedString>,
    search: Option<Task<()>>,
    generation: u64,
    pub(super) errors: BTreeMap<String, SharedString>,
    _subscriptions: Vec<Subscription>,
}
impl ThemeBrowser {
    pub(super) fn new(
        registry: Option<Arc<dyn ThemeRegistry>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search Zed themes"));
        let changed = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => this.search(true, false, cx),
            InputEvent::PressEnter { .. } => this.search(false, false, cx),
            _ => {}
        });
        let mut subscriptions = vec![
            changed,
            cx.observe_global::<super::Operations>(|_, cx| cx.notify()),
        ];
        if cx.has_global::<Themes>() {
            subscriptions.push(cx.observe_global::<Themes>(|_, cx| cx.notify()));
        }
        Self {
            registry,
            input,
            results: Vec::new(),
            searched: false,
            busy: false,
            error: None,
            search: None,
            generation: 0,
            errors: BTreeMap::new(),
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn search(&mut self, debounce: bool, browse: bool, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.search = None;
        let Some(registry) = self.registry.clone() else {
            self.error =
                Some("Theme registry is unavailable. Add JSON files to the themes folder.".into());
            cx.notify();
            return;
        };
        let query = if browse {
            String::new()
        } else {
            self.input.read(cx).value().to_string()
        };
        self.error = None;
        self.busy = true;
        self.searched = true;
        self.search = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
            }
            let result = cx
                .background_executor()
                .spawn(async move { registry.search(&query) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(results) => {
                        this.results = results.into_iter().take(50).collect();
                        this.error = None;
                    }
                    Err(e) => {
                        this.error = Some(format!("Could not search Zed: {e}. Try again.").into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn install(&mut self, extension: ExtensionInfo, cx: &mut Context<Self>) {
        if super::busy(&extension.id, cx) {
            return;
        }
        let Some(registry) = self.registry.clone() else {
            return;
        };
        let Some(themes) = cx.themes() else {
            return;
        };
        let directory = themes.dirs.installed.clone();
        let id = extension.id.clone();
        self.errors.remove(&id);
        super::begin(id.clone(), cx);
        let work = cx.background_executor().spawn(async move {
            let bytes = registry.download(&extension)?;
            install_archive(&directory, &extension, &bytes)
        });
        cx.spawn(async move |this, cx| {
            match work.await {
                Ok(()) => cx.update(reload_themes).await,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.errors.insert(
                            id.clone(),
                            format!("Could not install: {error}. Try again.").into(),
                        );
                        cx.notify();
                    });
                }
            }
            cx.update(|cx| super::end(&id, cx));
        })
        .detach();
        cx.notify();
    }
    pub(super) fn status(&self, extension: &ExtensionInfo, cx: &App) -> (&'static str, bool) {
        if super::busy(&extension.id, cx) {
            return ("Working…", true);
        }
        match cx
            .themes()
            .and_then(|t| t.catalog.packs().iter().find(|p| p.id == extension.id))
        {
            Some(pack) if pack.version == extension.version => ("Installed", true),
            Some(_) => ("Update", false),
            None => ("Install", false),
        }
    }
}
impl Render for ThemeBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content=v_flex().w_full().gap_2().py_3()
            .child(form::note("Search sends your query to zed.dev. Browse and Install connect only when you choose them.",cx))
            .child(h_flex().gap_2().child(gpui_kit::div().flex_1().min_w_0().child(Input::new(&self.input).small()))
                .child(Button::new("browse-zed-themes").small().ghost().label("Browse").disabled(self.registry.is_none()).on_click(cx.listener(|this,_,_,cx|this.search(false,true,cx)))))
            .when(self.busy,|el|el.child(form::note("Searching Zed…",cx)))
            .when_some(self.error.clone(),|el,error|el.child(form::error_text(error,cx)));
        if !self.searched {
            content = content.child(form::note(
                "Browse popular themes or search by name. No themes are downloaded automatically.",
                cx,
            ));
        } else if !self.busy && self.error.is_none() && self.results.is_empty() {
            content = content.child(form::note("No themes found. Try a different name.", cx));
        }
        for extension in &self.results {
            let (label, disabled) = self.status(extension, cx);
            let description = format!(
                "{} · {} downloads\n{}",
                extension.authors.join(", "),
                extension.download_count,
                extension.description.as_deref().unwrap_or_default()
            );
            let install = extension.clone();
            content = content.child(
                v_flex()
                    .child(form::row(
                        extension.name.clone(),
                        description,
                        Button::new(SharedString::from(format!(
                            "install-theme-{}",
                            extension.id
                        )))
                        .small()
                        .ghost()
                        .label(label)
                        .disabled(disabled)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.install(install.clone(), cx)),
                        ),
                        cx,
                    ))
                    .when_some(self.errors.get(&extension.id).cloned(), |el, error| {
                        el.child(form::error_text(error, cx))
                    }),
            );
        }
        if self.results.len() == 50 {
            content = content.child(form::note(
                "Showing the first 50 themes. Search to narrow the results.",
                cx,
            ));
        }
        content
    }
}
