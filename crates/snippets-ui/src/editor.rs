//! A focused draft with native code highlighting and explicit binding choices.
mod bindings;
mod draft;
mod picker;
mod render;
use crate::Installed;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, Subscription, WeakEntity, Window,
    component::{WindowExt as _, input::InputEvent},
    prelude::*,
    px,
};
use nocterm_snippets::{Language, Snippet};
use nocterm_workspace::Workspace;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) fn open(
    installed: Installed,
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
    let editor =
        cx.new(|cx| SnippetEditor::new(installed.clone(), original, workspace, window, cx));
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
            .bg(nocterm_ui::hsla(
                nocterm_ui::TerminalStyle::with_settings(
                    installed
                        .settings()
                        .setting::<nocterm_ui::TerminalSettings>(cx),
                    cx,
                )
                .background,
            ))
            .child(content.clone())
            .footer(footer.clone())
            .on_close(move |_, _, _| dismissed.store(true, Ordering::Release))
            // Both text areas use Enter to insert new lines; save is explicit.
            .on_ok(|_, _, _| false)
    });
    // The toolkit focuses the new modal root before rendering its children.
    // Keep that owner handle so a save cannot dismiss a newer stacked dialog.
    let dialog_focus = window.focused(cx);
    editor.update(cx, |editor, _| editor.dialog_focus = dialog_focus);
    let focus = editor.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    editor
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorSection {
    Snippet,
    Attachments,
}

pub(crate) struct SnippetEditor {
    draft: draft::SnippetDraft,
    picker: picker::AttachmentPicker,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    pending: bool,
    saved: bool,
    dialog_focus: Option<FocusHandle>,
    dismissed: Arc<AtomicBool>,
    _subscriptions: Vec<Subscription>,
    installed: Installed,
}
impl SnippetEditor {
    fn new(
        installed: Installed,
        original: Option<Snippet>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = original.clone().unwrap_or_else(|| installed.new_draft());
        let draft = draft::SnippetDraft::new(original, draft, window, cx);
        let picker = picker::AttachmentPicker::new(window, cx);
        let subscriptions = vec![
            cx.observe(picker.search(), |_, _, cx| cx.notify()),
            cx.subscribe(draft.name(), |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_draft_changed(cx);
                }
            }),
            cx.subscribe(draft.description(), |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_draft_changed(cx);
                }
            }),
            cx.subscribe(draft.code(), |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_draft_changed(cx);
                }
            }),
        ];
        Self {
            draft,
            picker,
            workspace,
            focus: cx.focus_handle(),
            pending: false,
            saved: false,
            dialog_focus: None,
            dismissed: Default::default(),
            _subscriptions: subscriptions,
            installed,
        }
    }
    fn on_draft_changed(&mut self, cx: &mut Context<Self>) {
        self.draft.edited();
        self.saved = false;
        cx.notify();
    }

    fn select_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.saved = false;
        self.draft.select_language(language, cx);
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.dismissed.load(Ordering::Acquire) {
            return;
        }
        let snippet = match self.draft.candidate(cx) {
            Ok(snippet) => snippet,
            Err(error) => {
                let focus = self.draft.validation_focus(&error, cx);
                self.draft.failed(error.to_string().into());
                if let Some(focus) = focus {
                    self.picker.select(EditorSection::Snippet);
                    self.focus_after_render(EditorSection::Snippet, focus, window, cx);
                }
                cx.notify();
                return;
            }
        };
        let saved = self
            .installed
            .save(snippet.clone(), self.draft.original().cloned(), cx);
        self.pending = true;
        self.saved = false;
        self.draft.edited();
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
                        this.draft.failed(error.to_string().into());
                        cx.notify();
                    }
                    Ok(()) => {
                        this.draft.accepted(snippet);
                        this.saved = true;
                        if this
                            .dialog_focus
                            .as_ref()
                            .is_some_and(|focus| focus.contains_focused(window, cx))
                        {
                            this.dismissed.store(true, Ordering::Release);
                            window.close_dialog(cx);
                        } else {
                            // A newer modal has focus. Leave it intact and keep this saved
                            // draft available to close or continue editing when it resurfaces.
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }
    fn select_section(
        &mut self,
        section: EditorSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.select(section);
        let focus = match section {
            EditorSection::Snippet => self.draft.focus(cx),
            EditorSection::Attachments => self.picker.focus(cx),
        };
        self.focus_after_render(section, focus, window, cx);
        cx.notify();
    }
    fn focus_after_render(
        &self,
        section: EditorSection,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if editor.upgrade().is_some_and(|editor| {
                editor.read(cx).picker.selected() == section
                    && !editor.read(cx).dismissed.load(Ordering::Acquire)
            }) {
                window.focus(&focus, cx);
            }
        });
    }
    fn set_binding(&mut self, group: bool, id: String, checked: bool, cx: &mut Context<Self>) {
        self.draft.set_binding(group, id, checked);
        self.saved = false;
        cx.notify();
    }
}
impl Focusable for SnippetEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.draft.focus(cx)
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
