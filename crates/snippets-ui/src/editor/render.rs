use super::SnippetEditor;
use gpui_kit::{
    AnyElement, Context, SharedString, Window,
    component::{
        ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex,
        input::{Editor, Input, Textarea},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_snippets::Language;
use nocterm_ui::form;
use std::sync::atomic::Ordering;
impl SnippetEditor {
    fn bindings(&self, cx: &mut Context<Self>) -> AnyElement {
        let directory = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).connection_directory());
        let connections = directory
            .as_ref()
            .map_or_else(Vec::new, |directory| directory.connections(cx));
        let groups = directory
            .as_ref()
            .map_or_else(Vec::new, |directory| directory.groups(cx));
        let profiles = connections
            .iter()
            .map(|connection| (connection.id.to_string(), connection.name.to_string()))
            .chain(
                self.draft
                    .profiles
                    .iter()
                    .filter(|id| {
                        !connections
                            .iter()
                            .any(|connection| connection.id.as_ref() == id.as_str())
                    })
                    .map(|id| (id.clone(), format!("Unavailable server ({id})"))),
            );
        let groups = groups
            .iter()
            .map(|name| (name.to_string(), name.to_string()))
            .chain(
                self.draft
                    .groups
                    .iter()
                    .filter(|name| !groups.iter().any(|group| group.as_ref() == name.as_str()))
                    .map(|name| (name.clone(), format!("Unavailable group: {name}"))),
            );
        let mut servers = v_flex()
            .id("snippet-servers")
            .w_full()
            .max_h(px(128.))
            .overflow_y_scroll()
            .gap_2()
            .py_1();
        let mut folders = v_flex()
            .id("snippet-groups")
            .w_full()
            .max_h(px(128.))
            .overflow_y_scroll()
            .gap_2()
            .py_1();
        let mut server_count = 0;
        let mut group_count = 0;
        for (id, name) in profiles {
            server_count += 1;
            let checked = self.draft.profiles.contains(&id);
            servers = servers.child(
                Checkbox::new(SharedString::from(format!("profile-{id}")))
                    .small()
                    .label(name)
                    .checked(checked)
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, checked, _, cx| {
                        this.set_binding(false, id.clone(), *checked, cx)
                    })),
            );
        }
        for (id, name) in groups {
            group_count += 1;
            let checked = self.draft.groups.contains(&id);
            folders = folders.child(
                Checkbox::new(SharedString::from(format!("group-{id}")))
                    .small()
                    .label(name)
                    .checked(checked)
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, checked, _, cx| {
                        this.set_binding(true, id.clone(), *checked, cx)
                    })),
            );
        }
        h_flex()
            .w_full()
            .items_start()
            .gap_6()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Servers"))
                    .child(servers)
                    .when(server_count == 0, |column| {
                        column.child(form::note("No saved servers.", cx))
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_sm().font_medium().child("Groups"))
                    .child(folders)
                    .when(group_count == 0, |column| {
                        column.child(form::note("No saved groups.", cx))
                    }),
            )
            .into_any_element()
    }
    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        v_flex()
            .gap_2()
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
                            .label("Cancel")
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
impl Render for SnippetEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex().id("snippet-editor").w_full().max_h(px(660.)).overflow_y_scroll().gap_3()
            .child(v_flex().gap_1().child(div().text_sm().font_medium().child("Name")).child(Input::new(&self.name).disabled(self.pending)))
            .child(v_flex().gap_1().child(div().text_sm().font_medium().child("Description")).child(Textarea::new(&self.description).h(px(64.)).disabled(self.pending)))
            .child(v_flex().gap_2().child(h_flex().gap_2().flex_wrap().items_center().child(div().text_sm().font_medium().child("Code"))
                .children(Language::ALL.into_iter().map(|language| Button::new(language.id()).ghost().xsmall().label(language.label()).selected(self.draft.language == language).disabled(self.pending).on_click(cx.listener(move |this, _, _, cx| this.select_language(language, cx))))))
                .child(Editor::new(&self.code).h(px(220.)).disabled(self.pending).aria_label("Snippet code")))
            .child(div().pt_1().text_xs().text_color(theme.muted_foreground).child("Assign to servers or groups to show this snippet first. Unassigned snippets stay in All other snippets."))
            .child(self.bindings(cx))
    }
}
