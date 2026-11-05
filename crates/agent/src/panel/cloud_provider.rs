//! Explicit personal-agent selection for a reopened own-runner chat. The captured composer
//! owns key admission; this view owns only its cancellable listing and choice presentation.
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, SharedString, Task, TestSupportExt as _,
    Window,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        popover::Popover,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_agent_chat::conversation::{ConversationComposer, ConversationProvider};
use std::rc::Rc;

pub(super) struct SourceProviderControl {
    composer: Rc<dyn ConversationComposer>,
    open: bool,
    choices: Option<Vec<ConversationProvider>>,
    work: Option<Task<()>>,
    error: Option<String>,
}
impl SourceProviderControl {
    pub(super) fn install(composer: Rc<dyn ConversationComposer>, cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self {
            composer,
            open: false,
            choices: None,
            work: None,
            error: None,
        })
    }
    fn toggle(&mut self, open: bool, cx: &mut Context<Self>) {
        self.open = open;
        if !open || self.work.is_some() {
            cx.notify();
            return;
        }
        self.choices = None;
        self.error = None;
        let listing = self.composer.providers(cx);
        self.work = Some(cx.spawn(async move |control, cx| {
            let result = listing.await;
            let _ = control.update(cx, |control, cx| {
                control.work = None;
                match result {
                    Ok(choices) => control.choices = Some(choices),
                    Err(error) => control.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose(&mut self, id: String, cx: &mut Context<Self>) {
        if self.work.is_some() {
            return;
        }
        self.error = None;
        let selection = self.composer.select_provider(id, cx);
        self.work = Some(cx.spawn(async move |control, cx| {
            let result = selection.await;
            let _ = control.update(cx, |control, cx| {
                control.work = None;
                match result {
                    Ok(()) => control.open = false,
                    Err(error) => control.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex()
            .id("cloud-providers")
            .test_support()
            .w(rems(18.))
            .max_h(rems(15.))
            .overflow_y_scroll()
            .gap_1();
        body = body.child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .child("Choose whose provider key this chat may use."),
        );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .id("cloud-provider-error")
                    .test_support()
                    .px_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if self.error.is_some() && self.work.is_none() {
            body = body.child(
                Button::new("cloud-provider-reload")
                    .small()
                    .label("Reload agents")
                    .on_click(cx.listener(|control, _, _, cx| control.toggle(true, cx))),
            );
        }
        if self.work.is_some() {
            body = body.child(
                div()
                    .id("cloud-provider-loading")
                    .test_support()
                    .px_2()
                    .text_sm()
                    .child("Connecting personal agent…"),
            );
        }
        if let Some(choices) = &self.choices {
            if choices.is_empty() {
                body = body.child(div().id("cloud-provider-empty").test_support().px_2().text_sm().child("No personal agents with saved keys. Add one in Servers → Cloud agents."));
            }
            for choice in choices {
                let id = choice.id.clone();
                body = body.child(
                    Button::new(SharedString::from(format!("cloud-provider-option-{id}")))
                        .ghost()
                        .small()
                        .max_w_full()
                        .label(choice.label.clone())
                        .disabled(self.work.is_some())
                        .on_click(
                            cx.listener(move |control, _, _, cx| control.choose(id.clone(), cx)),
                        ),
                );
            }
        }
        body.into_any_element()
    }
}
impl Render for SourceProviderControl {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let content = owner.clone();
        Popover::new("cloud-provider-popover")
            .anchor(gpui_kit::Anchor::BottomLeft)
            .p_1()
            .bg(cx.theme().muted)
            .open(self.open)
            .trigger_style(gpui_kit::StyleRefinement::default().min_w_0().max_w_full())
            .trigger(
                Button::new("cloud-provider-picker")
                    .ghost()
                    .small()
                    .max_w_full()
                    .label("Choose personal agent")
                    .accessibility_label("Choose personal agent")
                    .tooltip("Allow this chat to use a personal agent's provider key")
                    .dropdown_caret(true),
            )
            .on_open_change(move |open, _, cx| {
                let _ = owner.update(cx, |control, cx| control.toggle(*open, cx));
            })
            .content(move |_, _, cx| {
                content
                    .update(cx, |control, cx| control.choices(cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
    }
}
