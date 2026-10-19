//! The composer: attachments, images, the message field and the controls
//! under it.
use gpui_kit::{
    Anchor, AnyElement, App, ClipboardEntry, ClipboardItem, Context, Entity, SharedString,
    TestSupportExt as _, WeakEntity,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ai::model::auth::AuthFlow;
use nocterm_ui::IconName;

use super::{
    AgentPanel, MenuKind,
    widgets::{attachment_icon, menu_variant, mode_label, running_dot},
};
use crate::thread::{AgentThread, config_label};

fn remove_image(origin: gpui_kit::EntityId, index: usize, cx: &mut Context<AgentPanel>) -> Button {
    Button::new(("remove-image", index))
        .ghost()
        .xsmall()
        .icon(IconName::X)
        .tooltip("Remove image")
        .on_click(cx.listener(move |this, _, _, cx| {
            this.on_image_removed(origin, index, cx);
        }))
}

/// Pasted files and images become attachments; anything else is text.
pub(super) fn paste(
    panel: &WeakEntity<AgentPanel>,
    clipboard: &ClipboardItem,
    cx: &mut App,
) -> bool {
    let paths = clipboard
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    if !paths.is_empty() {
        let _ = panel.update(cx, |panel, cx| panel.add_images(paths, cx));
        return true;
    }
    let image = clipboard.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image.clone()),
        _ => None,
    });
    if let Some(image) = image {
        let _ = panel.update(cx, |panel, cx| panel.add_image_bytes(image.bytes, cx));
        return true;
    }
    false
}

pub(super) struct ComposerRenderer;
impl ComposerRenderer {
    /// Removable chips for the attached terminals and servers.
    pub(super) fn render_attachment_chips(
        thread: &Entity<AgentThread>,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        if thread.read(cx).composer().attachments().is_empty() {
            return None;
        }
        let origin = thread.entity_id();
        let mut chips = h_flex()
            .id("agent-attachment-chips")
            .test_support()
            .min_w_0()
            .gap_1()
            .flex_wrap()
            .items_start();
        let attachments = thread
            .read(cx)
            .composer()
            .attachments()
            .iter()
            .map(|attachment| attachment.to_attachment())
            .collect::<Vec<_>>();
        for attachment in &attachments {
            let value = attachment.clone();
            let label = labels.label(attachment);
            let running = labels.connected(attachment);
            chips = chips.child(
                Button::new(SharedString::from(format!("chip-{label}")))
                    .custom(menu_variant(cx))
                    .small()
                    .icon(attachment_icon(attachment))
                    .max_w_full()
                    .label(label.clone())
                    .tooltip(format!(
                        "Remove {label}{}",
                        if running { " · Running session" } else { "" }
                    ))
                    .when(running, |button| button.child(running_dot(cx)))
                    .child(gpui_kit::component::Icon::new(IconName::X).xsmall())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.on_attachment_selected(origin, value.clone(), cx);
                    })),
            );
        }
        Some(chips.into_any_element())
    }
    /// Thumbnails of the images the next message will carry.
    pub(super) fn render_image_strip(
        origin: gpui_kit::EntityId,
        previews: Vec<AnyElement>,
        cx: &mut Context<AgentPanel>,
    ) -> Option<AnyElement> {
        if previews.is_empty() {
            return None;
        }
        let mut strip = h_flex().id("agent-image-strip").flex_wrap().gap_2();
        for (index, thumbnail) in previews.into_iter().enumerate() {
            strip = strip.child(
                div()
                    .relative()
                    .size(rems(3.5))
                    .rounded(cx.theme().radius)
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(thumbnail)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .child(remove_image(origin, index, cx)),
                    ),
            );
        }
        Some(strip.into_any_element())
    }
    /// The message field, with slash-command tokens and image paste.
    pub(super) fn render_input(
        composer: &super::composer_view::ComposerView,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let command_panel = cx.weak_entity();
        let command_input = cx.weak_entity();
        composer
            .textarea()
            .appearance(false)
            .bordered(false)
            .token(move |token, _, cx| {
                let name = token.token().id().to_string();
                let owner = command_panel.clone();
                div()
                    .id("composer-command-token")
                    .test_support()
                    .h(token.line_height())
                    .rounded(px(4.))
                    .px_1()
                    .bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
                    .child(token.token().label().clone())
                    .hoverable_tooltip(move |_, cx| {
                        super::commands::tooltip(name.clone(), owner.clone(), cx)
                    })
            })
            .on_token_click(move |event, window, cx| {
                let _ = command_input.update(cx, |panel, cx| {
                    panel.on_command_token_clicked(event.range(), window, cx)
                });
            })
            .on_paste(move |clipboard, _, cx| paste(&weak, clipboard, cx))
            .into_any_element()
    }
    /// The session's configuration and mode pickers.
    pub(super) fn render_config_controls(
        _composer: &super::composer_view::ComposerView,
        menus: &super::menu_view::MenuView,
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let generating = thread.read(cx).session_generating();
        let mut controls = h_flex().w_full().min_w_0().gap_1().flex_wrap();
        for option in &thread.read(cx).config_options() {
            let id = option.id.to_string();
            controls = controls.child(super::menu::ConfigMenuRenderer::popover(
                menus,
                SharedString::from(format!("config-popover-{id}")),
                MenuKind::Config(id.clone()),
                Button::new(SharedString::from(format!("config-picker-{id}")))
                    .ghost()
                    .small()
                    .max_w_full()
                    .label(config_label(option))
                    .dropdown_caret(true)
                    .tooltip(super::commands::config_hint(option))
                    .disabled(generating),
                Anchor::BottomLeft,
                cx,
            ));
        }
        if !thread.read(cx).configuration().config_overrides_modes()
            && thread.read(cx).configuration().modes().is_some()
        {
            controls = controls.child(super::menu::ConfigMenuRenderer::popover(
                menus,
                "mode-popover",
                MenuKind::Modes,
                Button::new("mode-picker")
                    .ghost()
                    .small()
                    .max_w_full()
                    .label(mode_label(&thread.read(cx).configuration()))
                    .dropdown_caret(true)
                    .disabled(generating),
                Anchor::BottomLeft,
                cx,
            ));
        }
        controls.into_any_element()
    }
    /// Attach buttons, session controls, usage and send.
    pub(super) fn render_footer(
        composer: &super::composer_view::ComposerView,
        menus: &super::menu_view::MenuView,
        thread: &Entity<AgentThread>,
        usage: Option<&nocterm_ai::model::usage::ContextUsage>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let context_tooltip = format!(
            "{} attached terminals · sent metadata {} B · terminal output {} B",
            thread.read(cx).composer().attachments().len(),
            thread.read(cx).context_bytes(),
            thread.read(cx).tool_bytes()
        );
        let images_supported = thread
            .read(cx)
            .agent_info()
            .is_some_and(|info| info.capabilities.content.image);
        let controls = Self::render_config_controls(composer, menus, thread, cx);
        h_flex()
            .gap_1()
            .child(super::menu::ConfigMenuRenderer::popover(
                menus,
                "context-popover",
                MenuKind::Context,
                Button::new("agent-attach-context")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip(format!("Attach terminal or server\n{context_tooltip}")),
                Anchor::BottomLeft,
                cx,
            ))
            .child(
                Button::new("agent-attach-image")
                    .ghost()
                    .small()
                    .icon(IconName::ImagePlus)
                    .tooltip("Attach image")
                    .disabled(composer.preparing() || !images_supported)
                    .on_click(cx.listener(|this, _, _, cx| this.pick_images(cx))),
            )
            .child(div().flex_1().min_w_0().child(controls))
            .child(super::usage::UsageCard::usage_button(
                menus.is_open(&MenuKind::Usage),
                usage,
                cx,
            ))
            .child(Self::render_send(composer, thread, cx))
            .into_any_element()
    }
    /// Whether the send button stops the running turn instead.
    pub(super) fn stops(
        composer: &super::composer_view::ComposerView,
        thread: &Entity<AgentThread>,
        cx: &App,
    ) -> bool {
        thread.read(cx).session_generating()
            && composer.text(cx).trim().is_empty()
            && thread.read(cx).composer().images().is_empty()
            && !composer.editing()
    }
    pub(super) fn render_send(
        composer: &super::composer_view::ComposerView,
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let origin = thread.entity_id();
        let stop = Self::stops(composer, thread, cx);
        Button::new("agent-send")
            .primary()
            .small()
            .icon(if stop {
                IconName::CircleStop
            } else {
                IconName::ArrowUp
            })
            .rounded(px(999.))
            .accessibility_label(if stop {
                "Stop generation"
            } else {
                "Send message"
            })
            .flex_shrink_0()
            .tooltip(if stop { "Stop" } else { "Send" })
            .disabled(thread.read(cx).session_sign_in_required() || composer.preparing())
            .on_click(
                cx.listener(move |panel, _, window, cx| panel.on_send_clicked(origin, window, cx)),
            )
            .into_any_element()
    }
    /// Image preparation, restart and sign-in rows under the controls.
    pub(super) fn render_notices(
        composer: &super::composer_view::ComposerView,
        terminal: bool,
        thread: &Entity<AgentThread>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<AnyElement> {
        let origin = thread.entity_id();
        let mut notices = Vec::new();
        if composer.preparing() {
            notices.push(
                h_flex()
                    .gap_1()
                    .child(running_dot(cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Preparing images…"),
                    )
                    .into_any_element(),
            );
        }
        if thread.read(cx).ended() && !thread.read(cx).session_generating() {
            notices.push(
                Button::new("agent-restart")
                    .label("Restart chat")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.on_restart_clicked(origin, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        if thread.read(cx).session_sign_in_required()
            && let Some(info) = &thread.read(cx).agent_info()
        {
            for method in &info.auth_methods {
                if matches!(method.flow, AuthFlow::TerminalSetup(_)) && !terminal {
                    continue;
                }
                let id = method.id.clone();
                notices.push(
                    Button::new(SharedString::from(format!("authenticate-{id}")))
                        .ghost()
                        .small()
                        .label(format!("Sign in: {}", method.label))
                        .disabled(thread.read(cx).session_signing_in())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.on_authenticate_clicked(origin, id.clone(), window, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        notices
    }
}

impl ComposerRenderer {
    pub(super) fn compose(
        chips: Option<AnyElement>,
        images: Option<AnyElement>,
        input: AnyElement,
        footer: AnyElement,
        notices: Vec<AnyElement>,
        commands: Vec<usize>,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        v_flex()
            .id("agent-composer")
            .test_support()
            .when(!commands.is_empty(), |composer| {
                composer.key_context("AgentSlashCommands")
            })
            .m_3()
            .p_2()
            .gap_2()
            .min_w_0()
            .rounded(px(16.))
            .bg(cx.theme().muted)
            .border_1()
            .border_color(cx.theme().border)
            .children(chips)
            .children(images)
            .child(input)
            .child(footer)
            .children(notices)
            .into_any_element()
    }
}
