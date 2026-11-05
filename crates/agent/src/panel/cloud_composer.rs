//! Controls for a captured conversation's prompt draft; its source admits all changes.
use super::{AgentPanel, MenuKind, conversation_surface::ConversationSurface};
use gpui_kit::{
    Anchor, AnyElement, Context, EntityId, SharedString,
    component::{
        Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_agent_chat::conversation::ConversationComposer;
use nocterm_ui::IconName;
use std::rc::Rc;

pub(super) struct CloudComposerRenderer;
impl CloudComposerRenderer {
    pub(super) fn attach(
        selected: &ConversationSurface,
        preparing: bool,
        can_prompt: bool,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        let mut controls = Vec::new();
        let origin = selected.identity();
        if let Some(servers) = selected.servers() {
            controls.push(
                Button::new("agent-attach-context")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Add server")
                    .accessibility_label("Add server")
                    .on_click(move |_, _, cx| {
                        servers.update(cx, |control, cx| control.toggle_form(cx))
                    })
                    .into_any_element(),
            );
        }
        if selected.source().composer(cx).is_some() {
            controls.push(
                Button::new("agent-attach-image")
                    .ghost()
                    .small()
                    .icon(IconName::ImagePlus)
                    .tooltip("Attach image")
                    .accessibility_label("Attach image")
                    .disabled(preparing || !can_prompt)
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        if panel.conversation_identity() == Some(origin) {
                            panel.pick_images(cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        controls
    }
    pub(super) fn picker(
        selected: &ConversationSurface,
        menus: &super::menu_view::MenuView,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let composer = selected.source().composer(cx)?;
        let models = composer.models(cx);
        if models.is_empty() && composer.can_choose_provider(cx) {
            return selected
                .provider_picker()
                .map(|control| control.into_any_element());
        }
        let current = composer.selected_model(cx);
        let label = current
            .map(|model| model.model)
            .unwrap_or_else(|| "Choose model".into());
        Some(
            super::menu::ConfigMenuRenderer::popover(
                menus,
                "cloud-model-popover",
                MenuKind::CloudModels,
                Button::new("cloud-model-picker")
                    .ghost()
                    .small()
                    .max_w_full()
                    .label(label)
                    .accessibility_label("Choose model")
                    .tooltip("Choose the model for the next message. Agent model changes apply after its instance restarts.")
                    .dropdown_caret(models.len() > 1)
                    .disabled(models.len() < 2),
                Anchor::BottomLeft,
                cx,
            )
            .into_any_element(),
        )
    }
    pub(super) fn image_strip(
        selected: &ConversationSurface,
        messages: &mut super::message_list::MessageList,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        let source = selected.source();
        let composer = source.composer(cx)?;
        let images = composer.images(cx);
        let removals = (0..images.len())
            .map(|index| {
                super::conversation_surface::RenderedImageRemoval::capture(
                    source.as_ref(),
                    index,
                    cx,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let previews = images
            .into_iter()
            .map(|image| {
                super::preview_operation::PreviewOperation::cached(
                    messages,
                    super::image_pipeline::PreviewSource::Original(image.shared_bytes()),
                    None,
                    cx,
                )
            })
            .collect();
        super::composer_renderer::ComposerRenderer::render_image_strip(
            selected.identity(),
            previews,
            removals,
            cx,
        )
    }
    pub(super) fn models(
        origin: EntityId,
        composer: Rc<dyn ConversationComposer>,
        query: &str,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let selected = composer.selected_model(cx);
        let choices = composer
            .models(cx)
            .into_iter()
            .filter(|m| m.model.to_lowercase().contains(query))
            .collect::<Vec<_>>();
        let mut list = v_flex()
            .w(rems(18.))
            .max_h(rems(18.))
            .id("cloud-models")
            .overflow_y_scroll();
        if choices.is_empty() {
            list = list.child(div().px_2().py_1().text_sm().child("No matching models"));
        }
        for model in choices {
            let port = composer.clone();
            let active = selected.as_ref() == Some(&model);
            let value = model.clone();
            list = list.child(
                super::widgets::menu_row(
                    SharedString::from(format!("cloud-model-{}-{}", model.provider, model.model)),
                    model.model,
                    cx,
                )
                .selected(active)
                .on_click(cx.listener(move |panel, _, _, cx| {
                    if panel.conversation_identity() != Some(origin) {
                        return;
                    }
                    if let Err(error) = port.select_model(value.clone(), cx) {
                        panel.notices.set_error(Some(error.to_string()));
                    }
                    panel.close_menus(cx);
                })),
            );
        }
        list.child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .child("Agent model changes apply after its instance restarts."),
        )
        .into_any_element()
    }
}
