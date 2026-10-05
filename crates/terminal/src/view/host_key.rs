use super::*;

impl TerminalView {
    pub(super) fn render_host_key(
        &mut self,
        host: String,
        algorithm: String,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        self.card("Unknown host key", cx)
            .child(div().text_sm().child(format!(
                "This is the first connection to {host}. Check that the fingerprint \
                         below is the one the host's administrator gives you."
            )))
            .child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(algorithm),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .child(fingerprint),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("host-key-reject")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_host_key(HostKeyDecision::Reject, window, cx);
                            })),
                    )
                    .child(Button::new("host-key-once").label("Connect Once").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.answer_host_key(HostKeyDecision::AcceptOnce, window, cx);
                        }),
                    ))
                    .child(
                        Button::new("host-key-remember")
                            .primary()
                            .label("Trust and Connect")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_host_key(
                                    HostKeyDecision::AcceptAndRemember,
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }
}

pub(super) struct ChangedKey {
    pub host: String,
    pub algorithm: String,
    pub old_fingerprints: Vec<String>,
    pub fingerprint: String,
    pub known_hosts: std::path::PathBuf,
    pub line: usize,
    pub replacement_error: Option<String>,
}
impl TerminalView {
    pub(super) fn render_changed_host_key(
        &mut self,
        key: ChangedKey,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let unavailable = key.replacement_error.is_some();
        self.card("Host key changed", cx)
            .child(div().text_sm().child(format!(
                "The identity of {} has changed. This can happen after a server reinstall, or if someone intercepts the connection. Verify the new fingerprint with the host's administrator before connecting.", key.host
            )))
            .child(v_flex().id("changed-host-fingerprints").max_h(rems(12.)).overflow_y_scroll().overflow_x_scroll().gap_1().p_2().rounded(theme.radius).bg(theme.muted)
                .child(div().text_xs().text_color(theme.muted_foreground).child(key.algorithm))
                .child(div().text_xs().child("Previously trusted"))
                .children(key.old_fingerprints.into_iter().map(|fingerprint| div().text_sm().font_family(theme.mono_font_family.clone()).child(fingerprint)))
                .child(div().text_xs().child("New fingerprint"))
                .child(div().text_sm().font_family(theme.mono_font_family.clone()).child(key.fingerprint)))
            .child(div().text_xs().text_color(theme.muted_foreground).child(format!("Conflicting record: {} (line {})", key.known_hosts.display(), key.line)))
            .children(key.replacement_error.map(|reason| div().text_sm().child(reason)))
            .child(h_flex().flex_wrap().justify_end().gap_2()
                .child(Button::new("host-key-reject").ghost().label("Cancel").on_click(cx.listener(|this, _, window, cx| {
                    this.answer_host_key(HostKeyDecision::Reject, window, cx);
                })))
                .child(Button::new("host-key-once").label("Connect Once").on_click(cx.listener(|this, _, window, cx| {
                    this.answer_host_key(HostKeyDecision::AcceptOnce, window, cx);
                })))
                .child(Button::new("host-key-remember").primary().disabled(unavailable).label("Trust New Key and Connect").on_click(cx.listener(|this, _, window, cx| {
                    this.answer_host_key(HostKeyDecision::AcceptAndRemember, window, cx);
                }))))
            .into_any_element()
    }
}
