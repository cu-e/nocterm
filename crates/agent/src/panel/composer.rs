//! The composer: attachments, images, the message field and the controls
//! under it.
use gpui_kit::{
    Anchor, AnyElement, Context, Entity, Focusable as _, SharedString, TestSupportExt as _,
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
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_composer(
        &mut self,
        thread: &Entity<AgentThread>,
        state: &ThreadState,
        labels: &super::attachments::AttachmentLabels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stop_button = thread.read(cx).generating
            && self.input.read(cx).value().trim().is_empty()
            && thread.read(cx).images.is_empty()
            && self.composer.edit.is_none();
        let mut composer = v_flex()
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
            .border_color(cx.theme().border);
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
                        .disabled(thread.read(cx).generating),
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
                        .disabled(thread.read(cx).generating),
                    Anchor::BottomLeft,
                    cx,
                ),
            );
        }
        let context_tooltip = format!(
            "{} attached terminals · sent metadata {} B · terminal output {} B",
            thread.read(cx).attachments.len(),
            thread.read(cx).context_bytes,
            thread.read(cx).tool_bytes
        );
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
        if !thread.read(cx).attachments.is_empty() {
            composer = composer.child(chips);
        }
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
        if !previews.is_empty() {
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
                            div().absolute().top_0().right_0().child(
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
                                    })),
                            ),
                        ),
                );
            }
            composer = composer.child(strip);
        }
        let weak = cx.weak_entity();
        let command_panel = cx.weak_entity();
        let command_input = self.input.clone();
        composer = composer.child(
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
                .on_paste(move |clipboard, _, cx| {
                    let paths = clipboard
                        .entries()
                        .iter()
                        .filter_map(|entry| {
                            if let gpui_kit::ClipboardEntry::ExternalPaths(paths) = entry {
                                Some(paths.paths().to_vec())
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .collect::<Vec<_>>();
                    if !paths.is_empty() {
                        let _ = weak.update(cx, |panel, cx| panel.add_images(paths, cx));
                        return true;
                    }
                    if let Some(image) = clipboard.entries().iter().find_map(|entry| {
                        if let gpui_kit::ClipboardEntry::Image(image) = entry {
                            Some(image.clone())
                        } else {
                            None
                        }
                    }) {
                        let _ = weak.update(cx, |panel, cx| panel.add_image_bytes(image.bytes, cx));
                        return true;
                    }
                    false
                }),
        );
        composer = composer.child(
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
                        .disabled(
                            self.preparing_images()
                                || !thread.read(cx).info.as_ref().is_some_and(|info| {
                                    info.capabilities.prompt_capabilities.image
                                }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.pick_images(cx))),
                )
                .child(div().flex_1().min_w_0().child(controls))
                .child(self.usage_button(state.usage.as_ref(), cx))
                .child(
                    Button::new("agent-send")
                        .primary()
                        .small()
                        .icon(if stop_button {
                            IconName::CircleStop
                        } else {
                            IconName::ArrowUp
                        })
                        .rounded(px(999.))
                        .accessibility_label(if stop_button {
                            "Stop generation"
                        } else {
                            "Send message"
                        })
                        .flex_shrink_0()
                        .tooltip(if stop_button { "Stop" } else { "Send" })
                        .disabled(thread.read(cx).auth_required || self.preparing_images())
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.current().is_some_and(|thread| {
                                thread.read(cx).generating
                                    && this.input.read(cx).value().trim().is_empty()
                                    && thread.read(cx).images.is_empty()
                                    && this.composer.edit.is_none()
                            }) {
                                if let Some(thread) = this.current() {
                                    thread.update(cx, |thread, cx| thread.stop(cx));
                                }
                            } else {
                                this.send(window, cx);
                            }
                        })),
                ),
        );
        if self.preparing_images() {
            composer = composer.child(
                h_flex().gap_1().child(running_dot(cx)).child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Preparing images…"),
                ),
            );
        }
        if thread.read(cx).ended() && !thread.read(cx).generating {
            composer = composer.child(
                Button::new("agent-restart")
                    .label("Restart chat")
                    .on_click(cx.listener(|this, _, window, cx| this.restart(window, cx))),
            );
        }
        if thread.read(cx).auth_required
            && let Some(info) = &thread.read(cx).info
        {
            for method in &info.auth_methods {
                if matches!(method, acp::AuthMethod::Terminal(_))
                    && crate::TerminalAuth::opener(cx).is_none()
                {
                    continue;
                }
                let id = method.id().clone();
                composer = composer.child(
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
                        })),
                );
            }
        }
        composer.into_any_element()
    }
}
