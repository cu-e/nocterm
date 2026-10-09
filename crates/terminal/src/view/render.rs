use super::*;
use gpui_kit::base::TestSupportExt as _;

impl TerminalView {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn render_find(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let field = self.find.as_ref()?;
        let find = self.terminal.read(cx).find();
        let label = if let Some(error) = &find.error {
            error.clone()
        } else if find.query.is_empty() {
            "".into()
        } else if find.searching {
            "Searching…".into()
        } else if find.result.count == 0 {
            "No matches".into()
        } else {
            format!("{} of {}", find.result.ordinal, find.result.count)
        };
        let disabled = find.query.is_empty()
            || find.searching
            || find.result.count == 0
            || find.error.is_some();
        Some(
            v_flex()
                .flex_shrink_0()
                .px_2()
                .py_1()
                .gap_1()
                .bg(cx.theme().background)
                .on_action(cx.listener(|this, _: &native_input::Escape, window, cx| {
                    cx.stop_propagation();
                    this.hide_find(window, cx);
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.hide_find(window, cx);
                    }
                }))
                .child(
                    h_flex()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&field.input).small()),
                        )
                        .child(
                            Button::new("find-regex")
                                .ghost()
                                .small()
                                .label(".*")
                                .selected(find.options.regex)
                                .tooltip("Use regular expression (per logical line)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_find_option(
                                        |options| options.regex = !options.regex,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("find-case-sensitive")
                                .ghost()
                                .small()
                                .label("Aa")
                                .selected(find.options.case_sensitive)
                                .tooltip("Match case")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_find_option(
                                        |options| options.case_sensitive = !options.case_sensitive,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("find-whole-word")
                                .ghost()
                                .small()
                                .label("ab")
                                .selected(find.options.whole_word)
                                .tooltip("Match whole word")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_find_option(
                                        |options| options.whole_word = !options.whole_word,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("find-previous")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowUp)
                                .tooltip("Previous match (Shift+Enter)")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.terminal.update(cx, |t, cx| t.find_next(true, cx));
                                })),
                        )
                        .child(
                            Button::new("find-next")
                                .ghost()
                                .small()
                                .icon(IconName::ArrowDown)
                                .tooltip("Next match (Enter)")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.terminal.update(cx, |t, cx| t.find_next(false, cx));
                                })),
                        )
                        .child(
                            Button::new("find-close")
                                .ghost()
                                .small()
                                .icon(IconName::X)
                                .tooltip("Close search (Escape)")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.hide_find(window, cx)),
                                ),
                        ),
                )
                .when(!label.is_empty(), |bar| {
                    bar.child(
                        div()
                            .text_xs()
                            .min_w_0()
                            .truncate()
                            .text_color(if find.error.is_some() {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(label),
                    )
                })
                .into_any_element(),
        )
    }

    // ── Prompts ──────────────────────────────────────────────────────────────
}

impl TerminalView {
    fn render_prompt(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = match self.terminal.read(cx).prompt()? {
            Prompt::UnknownHostKey {
                host,
                algorithm,
                fingerprint,
                ..
            } => {
                let (host, algorithm, fingerprint) =
                    (host.clone(), algorithm.clone(), fingerprint.clone());
                self.render_host_key(host, algorithm, fingerprint, cx)
            }
            Prompt::ChangedHostKey {
                host,
                port,
                algorithm,
                old_fingerprints,
                fingerprint,
                known_hosts,
                line,
                replacement_error,
                ..
            } => {
                let details = host_key::ChangedKey {
                    host: if *port == 22 {
                        host.clone()
                    } else {
                        format!("[{host}]:{port}")
                    },
                    algorithm: algorithm.clone(),
                    old_fingerprints: old_fingerprints.clone(),
                    fingerprint: fingerprint.clone(),
                    known_hosts: known_hosts.clone(),
                    line: *line,
                    replacement_error: replacement_error.clone(),
                };
                self.render_changed_host_key(details, cx)
            }
            Prompt::Secret { request, .. } => {
                let request = request.clone();
                self.render_secret(&request, cx)?
            }
        };

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().overlay)
                .occlude()
                .child(card)
                .into_any_element(),
        )
    }

    pub(super) fn card(&self, title: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
        let theme = cx.theme();
        v_flex()
            .w(rems(cx.design().layout.dialog_width))
            .max_w_full()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .child(div().font_semibold().child(title.into()))
    }

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let terminal = self.terminal.read(cx);
        if terminal.prompt().is_some() {
            return None;
        }
        let target = terminal.spec().target.to_string();
        let theme = cx.theme();

        match terminal.status().clone() {
            Status::Connected => None,
            Status::Connecting(stage) => {
                let text = match stage {
                    ConnectStage::Connecting => format!("Connecting to {target}…"),
                    ConnectStage::Authenticating => format!("Signing in to {target}…"),
                    ConnectStage::StartingShell => "Starting the shell…".to_owned(),
                };
                Some(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(Icon::new(IconName::Loader).small())
                                .child(text),
                        )
                        .into_any_element(),
                )
            }
            Status::Closed(reason) => {
                let (icon, color, text) = match &reason {
                    CloseReason::Exited(None | Some(0)) => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        "The session ended.".to_owned(),
                    ),
                    CloseReason::Exited(Some(code)) => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        format!("The shell exited with status {code}."),
                    ),
                    CloseReason::ClosedByUser => (
                        IconName::Unplug,
                        theme.muted_foreground,
                        "The session was closed.".to_owned(),
                    ),
                    CloseReason::Failed(error) => (
                        IconName::ServerOff,
                        theme.danger,
                        capitalize(&error.to_string()),
                    ),
                };
                Some(
                    h_flex()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .justify_center()
                        .p_3()
                        .child(
                            h_flex()
                                .occlude()
                                .max_w_full()
                                .gap_3()
                                .py_2()
                                .px_3()
                                .rounded(theme.radius_lg)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .text_color(theme.popover_foreground)
                                .shadow_lg()
                                .child(Icon::new(icon).small().text_color(color))
                                .child(div().text_sm().min_w_0().child(text))
                                .child(
                                    Button::new("reconnect")
                                        .primary()
                                        .small()
                                        .icon(IconName::Plug)
                                        .label("Reconnect")
                                        .disabled(self.terminal.read(cx).is_command())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.reconnect(&Reconnect, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("close-ended-tab")
                                        .ghost()
                                        .small()
                                        .label("Close")
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(ItemEvent::CloseRequested)
                                        })),
                                ),
                        )
                        .into_any_element(),
                )
            }
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Render for TerminalView {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = TerminalStyle::current(cx);
        self.sync_palette(&style, cx);
        let background = nocterm_ui::hsla(style.background);

        let element = TerminalElement {
            view: cx.entity(),
            terminal: self.terminal.clone(),
            focus: self.focus_handle.clone(),
            style,
            focused: self.focus_handle.is_focused(window),
            cursor_lit: self.cursor_lit,
            marked_text: self.marked_text.clone().map(SharedString::from),
            frame: self.frame.clone(),
            frame_dirty: self.frame_dirty.clone(),
            frame_version: self.frame_version.clone(),
            highlights: self.highlights.clone(),
            geometry: self.geometry.clone(),
        };

        let grid = div()
            .id("terminal-drop-target")
            .test_support()
            .flex_1()
            .min_h_0()
            .key_context(KEY_CONTEXT)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .drag_over::<nocterm_workspace::FileDrag>(|style, _, _, cx| {
                style.border_1().border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(Self::drop_files))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::reconnect))
            .on_action(cx.listener(Self::toggle_recording))
            .child(
                self.on_edit_commands(div(), cx)
                    .key_context(SCREEN_KEY_CONTEXT)
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::paste))
                    .size_full()
                    .child(element.render()),
            )
            .children(self.render_prompt(cx))
            .children(self.render_status(cx));
        let terminal = self.terminal.read(cx);
        let recording = terminal.recording_status();
        let active = terminal.is_recording();
        let command = terminal.is_command();
        let connected = terminal.is_connected();
        let path = recording
            .and_then(|recording| recording.path)
            .map(|path| path.to_string_lossy().into_owned());
        self.on_commands(v_flex(), cx)
            .size_full()
            .min_h_0()
            .children(self.render_find(cx))
            .child(grid)
            .child(
                v_flex()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                Button::new("session-recording")
                                    .ghost()
                                    .small()
                                    .label(if active {
                                        "Stop recording"
                                    } else {
                                        "Record output"
                                    })
                                    .disabled(!connected || command)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.terminal.update(cx, |terminal, cx| {
                                            terminal.toggle_recording(cx)
                                        })
                                    })),
                            )
                            .when_some(path, |row, path| {
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(path),
                                )
                            }),
                    ),
            )
    }
}
