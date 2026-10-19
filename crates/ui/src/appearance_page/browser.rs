//! User-initiated Zed registry search and per-extension install feedback.
use crate::{ThemeUi, form};
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
use nocterm_themes::ExtensionInfo;
use std::time::Duration;

pub(super) struct ThemeBrowser {
    themes: ThemeUi,
    input: Entity<InputState>,
    results: Vec<ExtensionInfo>,
    searched: bool,
    busy: bool,
    error: Option<SharedString>,
    search: Option<Task<()>>,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}
impl ThemeBrowser {
    pub(super) fn new(themes: ThemeUi, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search Zed themes"));
        let changed = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => this.search(true, false, cx),
            InputEvent::PressEnter { .. } => this.search(false, false, cx),
            _ => {}
        });
        let view = cx.entity().downgrade();
        let observed = themes.themes().observe(cx, move |cx| {
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        let subscriptions = vec![changed, observed];
        Self {
            themes,
            input,
            results: Vec::new(),
            searched: false,
            busy: false,
            error: None,
            search: None,
            generation: 0,
            _subscriptions: subscriptions,
        }
    }
    pub(super) fn search(&mut self, debounce: bool, browse: bool, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.search = None;
        let Some(registry) = self.themes.themes().registry(cx) else {
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
        self.themes.themes().install_pack(extension, cx).detach();
    }
    pub(super) fn status(&self, extension: &ExtensionInfo, cx: &App) -> (&'static str, bool) {
        if self.themes.themes().busy(&extension.id, cx) {
            return ("Working…", true);
        }
        let catalog = self.themes.themes().catalog(cx);
        match catalog.packs().iter().find(|p| p.id == extension.id) {
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
                .child(Button::new("browse-zed-themes").small().ghost().label("Browse").disabled(self.themes.themes().registry(cx).is_none()).on_click(cx.listener(|this,_,_,cx|this.search(false,true,cx)))))
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
                    .when_some(
                        self.themes.themes().errors(cx).get(&extension.id).map(|e| {
                            SharedString::from(format!("Could not install: {e}. Try again."))
                        }),
                        |el, error| el.child(form::error_text(error, cx)),
                    ),
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
