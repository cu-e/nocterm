//! A focused draft with native code highlighting and explicit binding choices.
mod render;
use crate::model::Snippets;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window,
    component::{
        WindowExt as _,
        input::{EditorState, InputEvent, InputState, TextareaState},
    },
    prelude::*,
    px,
};
use nocterm_snippets::{Language, Snippet, ValidationError};
use nocterm_workspace::Workspace;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) fn open(
    original: Option<Snippet>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SnippetEditor> {
    let title = if original.is_some() {
        "Edit snippet"
    } else {
        "New snippet"
    };
    let editor = cx.new(|cx| SnippetEditor::new(original, workspace, window, cx));
    let footer = cx.new(|cx| EditorFooter {
        editor: editor.clone(),
        _subscription: cx.observe(&editor, |_, _, cx| cx.notify()),
    });
    let content = editor.clone();
    let dismissed = editor.read(cx).dismissed.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let dismissed = dismissed.clone();
        dialog
            .title(title)
            .w(px(720.))
            .bg(nocterm_ui::form::page_background(cx))
            .child(content.clone())
            .footer(footer.clone())
            .on_close(move |_, _, _| dismissed.store(true, Ordering::Release))
            // Both text areas use Enter to insert new lines; save is explicit.
            .on_ok(|_, _, _| false)
    });
    let focus = editor.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    editor
}
pub(crate) struct SnippetEditor {
    original: Option<Snippet>,
    draft: Snippet,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    code: Entity<EditorState>,
    workspace: WeakEntity<Workspace>,
    pending: bool,
    dismissed: Arc<AtomicBool>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}
impl SnippetEditor {
    fn new(
        original: Option<Snippet>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = original.clone().unwrap_or_default();
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("e.g. Inspect disk usage")
                .default_value(draft.name.clone())
        });
        let description =
            cx.new(|cx| TextareaState::new(window, cx).default_value(draft.description.clone()));
        let code = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(draft.language.id())
                .default_value(draft.content.clone())
        });
        let subscriptions = vec![
            cx.subscribe(&name, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&description, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&code, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    cx.notify();
                }
            }),
        ];
        Self {
            original,
            draft,
            name,
            description,
            code,
            workspace,
            pending: false,
            dismissed: Default::default(),
            error: None,
            _subscriptions: subscriptions,
        }
    }
    fn select_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.draft.language = language;
        self.code
            .update(cx, |code, cx| code.set_highlighter(language.id(), cx));
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.dismissed.load(Ordering::Acquire) {
            return;
        }
        let mut snippet = self.draft.clone();
        snippet.name = self.name.read(cx).value().trim().to_owned();
        snippet.description = self.description.read(cx).value().to_string();
        snippet.content = self.code.read(cx).value().to_string();
        if let Err(error) = snippet.validate() {
            let focus = match error {
                ValidationError::Name => Some(self.name.read(cx).focus_handle(cx)),
                ValidationError::Content => Some(self.code.read(cx).focus_handle(cx)),
                _ => None,
            };
            self.error = Some(error.to_string().into());
            if let Some(focus) = focus {
                window.focus(&focus, cx);
            }
            cx.notify();
            return;
        }
        let saved = Snippets::global(cx).update(cx, |model, cx| {
            model.save(snippet.clone(), self.original.clone(), cx)
        });
        self.pending = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.dismissed.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Err(error) => {
                        this.error = Some(error.into());
                        cx.notify();
                    }
                    Ok(()) => {
                        this.original = Some(snippet);
                        this.dismissed.store(true, Ordering::Release);
                        window.close_dialog(cx);
                    }
                }
            });
        })
        .detach();
    }
    fn set_binding(&mut self, group: bool, id: String, checked: bool, cx: &mut Context<Self>) {
        let bindings = if group {
            &mut self.draft.groups
        } else {
            &mut self.draft.profiles
        };
        if checked {
            if !bindings.contains(&id) {
                bindings.push(id);
            }
        } else {
            bindings.retain(|existing| existing != &id);
        }
        self.error = None;
        cx.notify();
    }
}
impl Focusable for SnippetEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }
}
struct EditorFooter {
    editor: Entity<SnippetEditor>,
    _subscription: Subscription,
}
impl Render for EditorFooter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.editor
            .update(cx, |editor, cx| editor.render_footer(cx))
    }
}
#[cfg(test)]
mod tests;
