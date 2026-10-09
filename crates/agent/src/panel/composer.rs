//! The composer: attachments, images, the message field and the controls
//! under it.
use gpui_kit::{
    Anchor, AnyElement, App, ClipboardEntry, ClipboardItem, Context, Entity, Focusable as _,
    SharedString, TestSupportExt as _, WeakEntity,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Textarea,
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ai::{acp, thread::ThreadState};
use nocterm_ui::IconName;

use super::{
    AgentPanel, MenuKind,
    widgets::{attachment_icon, menu_variant, mode_label, running_dot},
};
use crate::thread::{AgentThread, config_label};

impl AgentPanel {
    pub(super) fn render_composer(
        &mut self,
        thread: &Entity<AgentThread>,
        state: &ThreadState,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = self.render_attachment_chips(thread, labels, cx);
        let images = self.render_image_strip(thread, cx);
        let input = self.render_input(cx);
        let footer = self.render_footer(thread, state, cx);
        let notices = self.render_notices(thread, cx);
        v_flex()
            .id("agent-composer")
            .test_support()
            .when(!self.command_matches(cx).is_empty(), |composer| {
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

    /// The session's configuration and mode pickers.
    fn render_config_controls(
        &mut self,
        thread: &Entity<AgentThread>,
        state: &ThreadState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let generating = thread.read(cx).generating;
        let mut controls = h_flex().w_full().min_w_0().gap_1().flex_wrap();
        for option in &state.config_options {
            let id = option.id.to_string();
            controls = controls.child(
                self.menu_popover(
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
                ),
            );
        }
        if !state.config_overrides_modes() && state.modes.is_some() {
            controls = controls.child(
                self.menu_popover(
                    "mode-popover",
                    MenuKind::Modes,
                    Button::new("mode-picker")
                        .ghost()
                        .small()
                        .max_w_full()
                        .label(mode_label(state))
                        .dropdown_caret(true)
                        .disabled(generating),
                    Anchor::BottomLeft,
                    cx,
                ),
            );
        }
        controls.into_any_element()
    }

    /// Removable chips for the attached terminals and servers.
    fn render_attachment_chips(
        &self,
        thread: &Entity<AgentThread>,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if thread.read(cx).attachments.is_empty() {
            return None;
        }
        let mut chips = h_flex()
            .id("agent-attachment-chips")
            .test_support()
            .min_w_0()
            .gap_1()
            .flex_wrap()
            .items_start();
        for attachment in &thread.read(cx).attachments {
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
                        if let Some(thread) = this.current() {
                            thread.update(cx, |thread, cx| thread.attach(value.clone(), cx));
                        }
                    })),
            );
        }
        Some(chips.into_any_element())
    }

    /// Thumbnails of the images the next message will carry.
    fn render_image_strip(
        &mut self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let previews = thread
            .read(cx)
            .images
            .iter()
            .map(|image| {
                let key = (image.bytes().as_ptr() as usize, image.bytes().len(), false);
                (
                    key,
                    (!self.image_cache.contains_key(&key)).then(|| image.bytes().to_vec()),
                )
            })
            .collect::<Vec<_>>();
        if previews.is_empty() {
            return None;
        }
        let mut strip = h_flex().id("agent-image-strip").flex_wrap().gap_2();
        for (index, (key, data)) in previews.into_iter().enumerate() {
            let thumbnail = self.cached_image(key, || data.unwrap_or_default(), false, cx);
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
                            .child(remove_image(index, cx)),
                    ),
            );
        }
        Some(strip.into_any_element())
    }

    /// The message field, with slash-command tokens and image paste.
    fn render_input(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        let command_panel = cx.weak_entity();
        let command_input = self.input.clone();
        Textarea::new(&self.input)
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
                command_input.update(cx, |input, cx| {
                    input.set_selected_range(event.range(), cx);
                    window.focus(&input.focus_handle(cx), cx);
                });
            })
            .on_paste(move |clipboard, _, cx| paste(&weak, clipboard, cx))
            .into_any_element()
    }

    /// Attach buttons, session controls, usage and send.
    fn render_footer(
        &mut self,
        thread: &Entity<AgentThread>,
        state: &ThreadState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let context_tooltip = format!(
            "{} attached terminals · sent metadata {} B · terminal output {} B",
            thread.read(cx).attachments.len(),
            thread.read(cx).context_bytes,
            thread.read(cx).tool_bytes
        );
        let images_supported = thread
            .read(cx)
            .info
            .as_ref()
            .is_some_and(|info| info.capabilities.prompt_capabilities.image);
        let controls = self.render_config_controls(thread, state, cx);
        h_flex()
            .gap_1()
            .child(
                self.menu_popover(
                    "context-popover",
                    MenuKind::Context,
                    Button::new("agent-attach-context")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .tooltip(format!("Attach terminal or server\n{context_tooltip}")),
                    Anchor::BottomLeft,
                    cx,
                ),
            )
            .child(
                Button::new("agent-attach-image")
                    .ghost()
                    .small()
                    .icon(IconName::ImagePlus)
                    .tooltip("Attach image")
                    .disabled(self.preparing_images() || !images_supported)
                    .on_click(cx.listener(|this, _, _, cx| this.pick_images(cx))),
            )
            .child(div().flex_1().min_w_0().child(controls))
            .child(self.usage_button(state.usage.as_ref(), cx))
            .child(self.render_send(thread, cx))
            .into_any_element()
    }

    /// Whether the send button stops the running turn instead.
    fn stops(&self, thread: &Entity<AgentThread>, cx: &App) -> bool {
        thread.read(cx).generating
            && self.input.read(cx).value().trim().is_empty()
            && thread.read(cx).images.is_empty()
            && self.composer.edit.is_none()
    }

    fn render_send(&self, thread: &Entity<AgentThread>, cx: &mut Context<Self>) -> AnyElement {
        let stop = self.stops(thread, cx);
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
            .disabled(thread.read(cx).auth_required || self.preparing_images())
            .on_click(cx.listener(|this, _, window, cx| match this.current() {
                Some(thread) if this.stops(&thread, cx) => {
                    thread.update(cx, |thread, cx| thread.stop(cx));
                }
                _ => this.send(window, cx),
            }))
            .into_any_element()
    }

    /// Image preparation, restart and sign-in rows under the controls.
    fn render_notices(
        &self,
        thread: &Entity<AgentThread>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if self.preparing_images() {
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
        if thread.read(cx).ended() && !thread.read(cx).generating {
            notices.push(
                Button::new("agent-restart")
                    .label("Restart chat")
                    .on_click(cx.listener(|this, _, window, cx| this.restart(window, cx)))
                    .into_any_element(),
            );
        }
        if thread.read(cx).auth_required
            && let Some(info) = &thread.read(cx).info
        {
            let terminal = crate::TerminalAuth::opener(cx).is_some();
            for method in &info.auth_methods {
                if matches!(method, acp::AuthMethod::Terminal(_)) && !terminal {
                    continue;
                }
                let id = method.id().clone();
                notices.push(
                    Button::new(SharedString::from(format!("authenticate-{id}")))
                        .ghost()
                        .small()
                        .label(format!("Sign in: {}", method.name()))
                        .disabled(thread.read(cx).authenticating)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(thread) = this.current() {
                                thread.update(cx, |thread, cx| {
                                    thread.authenticate(id.clone(), window, cx)
                                });
                            }
                        }))
                        .into_any_element(),
                );
            }
        }
        notices
    }
}

fn remove_image(index: usize, cx: &mut Context<AgentPanel>) -> Button {
    Button::new(("remove-image", index))
        .ghost()
        .xsmall()
        .icon(IconName::X)
        .tooltip("Remove image")
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(thread) = this.current() {
                thread.update(cx, |thread, cx| {
                    if index < thread.images.len() {
                        thread.images.remove(index);
                    }
                    cx.notify();
                });
            }
        }))
}

/// Pasted files and images become attachments; anything else is text.
fn paste(panel: &WeakEntity<AgentPanel>, clipboard: &ClipboardItem, cx: &mut App) -> bool {
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
