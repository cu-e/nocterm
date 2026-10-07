use super::{EditorSection, SnippetEditor};
use gpui_kit::{
    AnyElement, Context, Window,
    base::TestSupportExt as _,
    component::{
        Disableable as _, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Editor, Input, Textarea},
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_snippets::Language;
use nocterm_ui::{ActiveDesign as _, IconName, form};
use std::sync::atomic::Ordering;
impl SnippetEditor {
    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_2()
            .when(self.saved, |footer| {
                footer.child(form::note("Snippet saved.", cx))
            })
            .when_some(self.error.clone(), |footer, error| {
                footer.child(form::error_text(error, cx))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("snippet-cancel")
                            .ghost()
                            .label(if self.saved { "Close" } else { "Cancel" })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dismissed.store(true, Ordering::Release);
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("snippet-save")
                            .primary()
                            .label(if self.pending {
                                "Saving…"
                            } else {
                                "Save snippet"
                            })
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
impl SnippetEditor {
    fn snippet_page(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(form::page_header(
                "Snippet",
                Some("Reusable commands and code, with language-aware highlighting.".into()),
                cx,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Name"))
                    .child(Input::new(&self.name).disabled(self.pending)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Description"))
                    .child(
                        Textarea::new(&self.description)
                            .h(px(64.))
                            .disabled(self.pending),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .child(div().text_sm().font_medium().child("Code"))
                            .children(Language::ALL.into_iter().map(|language| {
                                Button::new(language.id())
                                    .ghost()
                                    .xsmall()
                                    .label(language.label())
                                    .selected(self.draft.language == language)
                                    .disabled(self.pending)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_language(language, cx)
                                    }))
                            })),
                    )
                    .child(
                        Editor::new(&self.code)
                            .h(px(220.))
                            .disabled(self.pending)
                            .aria_label("Snippet code"),
                    ),
            )
            .into_any_element()
    }
}
impl Render for SnippetEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = &cx.design().layout;
        let height = rems(layout.connection_editor_height).to_pixels(window.rem_size());
        let available =
            (window.viewport_size().height - rems(10.).to_pixels(window.rem_size())).max(px(0.));
        let nav_width = rems(layout.connection_nav_width);
        let page = match self.section {
            EditorSection::Snippet => self.snippet_page(cx),
            EditorSection::Attachments => self.attachments_page(cx),
        };
        h_flex()
            .id("snippet-editor")
            .track_focus(&self.focus)
            .items_start()
            .h(height.min(available))
            .min_h_0()
            .gap_4()
            .child(
                v_flex().w(nav_width).flex_shrink_0().gap(px(2.)).children(
                    [
                        (
                            EditorSection::Snippet,
                            "snippet-section-snippet",
                            "Snippet",
                            IconName::ScrollText,
                        ),
                        (
                            EditorSection::Attachments,
                            "snippet-section-attachments",
                            "Attachments",
                            IconName::FolderTree,
                        ),
                    ]
                    .into_iter()
                    .map(|(section, id, label, icon)| {
                        form::nav_item(id, icon, label, self.section == section, cx)
                            .test_support()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_section(section, window, cx)
                            }))
                    }),
                ),
            )
            .child(
                v_flex()
                    .id("snippet-section-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .min_h_0()
                    .pr_2()
                    .overflow_y_scroll()
                    .child(page),
            )
    }
}
