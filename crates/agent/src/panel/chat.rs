use super::transcript::TranscriptView;
use super::usage::Report;
use super::*;
use super::{
    entries::{EntryRenderer, RowPresentation},
    widgets::entry_is_live,
};
use gpui_kit::AnyElement;
use nocterm_ai::usage::{Breakdown, RateLimit};

impl AgentPanel {
    /// The open chat's footer: approvals, status, queue, commands and the composer.
    pub(super) fn render_footer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let frame = super::composer_frame::ComposerFrame::read(&thread, cx);
        let approvals = self.render_approvals(&thread, window, cx);
        let status = self.chat_status(&thread, cx);
        let labels = attachments::AttachmentLabels::new(thread.read(cx).workspace_seat(), cx);
        let queue = self.render_queue(&thread, &labels, cx);
        let commands = self.render_commands(window, cx);
        let composer = self.render_composer(&frame, &thread, &labels, cx);
        v_flex()
            .w_full()
            .min_w_0()
            .children(approvals)
            .children(status)
            .children(queue)
            .children(commands)
            .child(composer)
            .into_any_element()
    }
    pub(super) fn sync_approval_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let attention = self
            .chats
            .iter()
            .any(|thread| thread.read(cx).has_pending_approvals());
        let needs_notice = attention
            && (self
                .current()
                .is_none_or(|thread| !thread.read(cx).has_pending_approvals())
                || self.host.workspace().upgrade().is_none_or(|workspace| {
                    !workspace.read(cx).panel_is_open(&crate::plugin::CHATS.id)
                }));
        // The workspace pulls the attention from `Panel::attention` when the panel notifies,
        // so notify only when it changed: this runs on every chunk a chat streams.
        let asks = Panel::attention(self, cx).is_some();
        if self.notices.attention_changed(asks) {
            cx.notify();
        }
        let requests = needs_notice.then(|| {
            self.chats
                .iter()
                .filter_map(|thread| {
                    let state = thread.read(cx);
                    state
                        .has_pending_approvals()
                        .then_some((thread.entity_id(), state.approval_notice_generation()))
                })
                .collect()
        });
        match self.notices.approval_changed(requests) {
            notices::ApprovalEffect::Unchanged => return,
            notices::ApprovalEffect::Remove => {
                nocterm_ui::notice::remove(window, cx, "agent-approval");
                return;
            }
            notices::ApprovalEffect::Show => {}
        }
        let panel = cx.weak_entity();
        let workspace = self.host.workspace().clone();
        nocterm_ui::notice::warning_action(
            window,
            cx,
            "agent-approval",
            "Agent request",
            "An agent needs permission to continue.",
            "Review",
            move |window, cx| {
                let _ = panel.update(cx, |panel, cx| panel.review_pending(cx));
                let _ = workspace.update(cx, |workspace, cx| {
                    if !workspace.panel_is_open(&crate::plugin::CHATS.id) {
                        workspace.toggle_panel(&crate::plugin::CHATS.id, window, cx);
                    }
                });
            },
        );
    }
    pub(super) fn review_pending(&mut self, cx: &mut Context<Self>) {
        let pending = |thread: &Entity<AgentThread>| thread.read(cx).has_pending_approvals();
        let owner = self
            .current()
            .filter(pending)
            .or_else(|| self.chats.iter().find(|thread| pending(thread)).cloned());
        if let Some(owner) = owner {
            if self
                .current()
                .is_none_or(|thread| thread.entity_id() != owner.entity_id())
            {
                self.open_thread(owner.entity_id(), cx);
            }
            cx.notify();
        }
    }
    /// The cards of every pending request of the chat: its sign-in, permission and tool
    /// approval requests; none when nothing waits.
    pub(super) fn render_approvals(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let mut list = Vec::new();
        {
            let asking: Vec<EntityId> = self
                .chats
                .iter()
                .flat_map(|thread| {
                    thread
                        .read(cx)
                        .sign_in_waits()
                        .iter()
                        .map(|wait| wait.item())
                })
                .collect();
            self.sign_ins.reconcile(&asking);
            for index in 0..thread.read(cx).sign_in_waits().len() {
                let wait = thread.read(cx).sign_in_waits()[index].clone();
                let (input, submission) = self.sign_ins.input_for(thread, &wait, window, cx);
                list.push(approvals::ApprovalCardsRenderer::sign_in_card(
                    wait, input, submission, index, cx,
                ));
            }
            for (index, permission) in thread.read(cx).permission_cards().into_iter().enumerate() {
                list.push(approvals::ApprovalCardsRenderer::permission_card(
                    thread, permission, index, cx,
                ));
            }
        }
        let tokens = thread.read(cx).tool_approval_tokens();
        for (index, token) in tokens.into_iter().enumerate() {
            if let Some(card) =
                approvals::ApprovalCardsRenderer::tool_card(thread, token, index, cx)
            {
                list.push(card);
            }
        }
        approvals::ApprovalCardsRenderer::list(list, window, cx)
    }
    pub(super) fn submit_sign_in(
        &mut self,
        submission: &sign_in::SignInSubmission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(answer) = submission.take_original_answer(window, cx) else {
            return;
        };
        if let Err(error) = submission.answer(answer, cx) {
            self.notices.set_error(Some(error.to_string()));
        }
        cx.notify();
    }
    pub(super) fn on_reveal_tick(
        &mut self,
        binding: message_list::ActiveListBinding,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.messages.thread() else {
            return;
        };
        let reduced = cx.reduce_motion();
        let changed =
            self.messages
                .advance_reveal(&binding, &thread.read(cx).transcript().entries, reduced);
        if let Some(changed) = changed {
            self.messages.apply_reveal(changed, cx);
            // The reveal animates the transcript's rows only.
            self.layout.parts().notify(parts::PartKind::Transcript, cx);
        }
    }
    pub(super) fn on_preview_completed(
        &mut self,
        binding: message_list::ActiveListBinding,
        ticket: image_pipeline::PreviewTicket,
        row: Option<usize>,
        image: Option<image_pipeline::CompletedPreview>,
        cx: &mut Context<Self>,
    ) {
        if self
            .messages
            .accept_preview(&binding, ticket, row, image, cx)
        {
            cx.notify();
        }
    }
    pub(super) fn on_message_expanded(
        &mut self,
        binding: Option<message_list::ActiveListBinding>,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(binding) = binding
            && self.messages.toggle_expanded(&binding, row, cx)
        {
            cx.notify();
        }
    }
    pub(super) fn chat_status(
        &self,
        thread: &Entity<AgentThread>,
        cx: &gpui_kit::App,
    ) -> Option<AnyElement> {
        let state = thread.read(cx);
        let visible = state.session_generating()
            || state.session().is_none()
            || state.session_sign_in_required()
            || state.document().pending_history()
            || state.has_status_error()
            || !state.session_accepts_updates();
        visible.then(|| {
            TranscriptView::status(
                thread.entity_id(),
                state.status(),
                state.session_generating() && state.session_accepts_updates(),
                cx,
            )
        })
    }
    /// The transcript of the open chat, for its cached view; the rows changed since its last
    /// drawing redraw.
    pub(super) fn render_transcript_part(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let dirty: Vec<usize> = thread
            .update(cx, |thread, _| thread.take_changed_rows())
            .into_iter()
            .collect();
        let stamp = parts::transcript_stamp(thread.read(cx));
        self.layout.parts_mut().drew_transcript(Some(stamp));
        self.observe_rows(&thread, &dirty, cx);
        let scroller = self.messages.scroller(cx);
        let navigation = navigation::NavigationRenderer::render(&self.messages, cx);
        let overlay = self
            .menus
            .is_open(&MenuKind::Usage)
            .then(|| self.render_usage(cx));
        let plan = thread
            .read(cx)
            .transcript()
            .plan
            .as_ref()
            .map(|plan| TranscriptView::plan(plan, cx));
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(TranscriptView::area(scroller, navigation, overlay, cx))
            .children(plan)
            .into_any_element()
    }
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(thread) = self.messages.thread() else {
            return div().into_any_element();
        };
        let identity = thread.entity_id();
        let markdown = thread
            .read(cx)
            .transcript()
            .entries
            .get(index)
            .and_then(|entry| {
                super::message_markdown::MessageMarkdown::text(entry, index, &self.messages)
            });
        if let Some((text, source_len)) = markdown {
            self.messages.prepare_markdown(index, text, source_len, cx);
        }
        let state = thread.read(cx);
        let transcript = state.transcript();
        let Some(entry) = transcript.entries.get(index) else {
            return div().into_any_element();
        };
        let generating = state.session_generating();
        let frame = RowPresentation::new(
            identity,
            self.messages.binding(),
            index,
            self.messages.expanded(index),
            entry_is_live(
                &transcript.entries,
                index,
                generating && state.session_accepts_updates(),
            ),
            generating,
            transcript.time(index),
            self.settings
                .setting::<nocterm_ai::AiSettings>(cx)
                .approval
                .redact_secrets,
        )
        .with_fork_available(true);
        let (row, pending) = EntryRenderer::render(frame, entry, &self.messages, cx);
        for source in pending {
            preview_operation::PreviewOperation::cached(
                &mut self.messages,
                source,
                Some(index),
                cx,
            );
        }
        row
    }
    /// The visible list follows the open chat. Each observer supplies its own dirty rows, so
    /// one window never consumes another's change tracking.
    pub(super) fn observe_rows(
        &mut self,
        thread: &Entity<AgentThread>,
        dirty: &[usize],
        cx: &mut Context<Self>,
    ) {
        self.messages.bind_thread(thread, cx);
        let reduced = cx.reduce_motion();
        let state = thread.read(cx);
        let rows = self.messages.observe_rows(
            thread.entity_id(),
            &state.transcript().entries,
            dirty,
            state.session_generating(),
            reduced,
        );
        self.messages.apply_rows(rows, cx);
    }
    pub(super) fn jump_to_message(&mut self, first: bool, cx: &mut Context<Self>) {
        self.messages.navigate(first, cx);
        cx.notify();
    }
}

impl AgentPanel {
    pub(super) fn dismiss_usage(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.menus.dismiss_usage() {
            return false;
        }
        cx.notify();
        true
    }
    pub(super) fn close_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let restore = self.menus.usage_contains_focus(window, cx);
        self.dismiss_usage(cx);
        if restore {
            window.focus(&self.composer.focus(cx), cx);
        }
    }
    pub(super) fn toggle_usage(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open {
            self.close_usage(window, cx);
        } else {
            self.menus.open(MenuKind::Usage);
            // Read-only information must not interrupt composing.
            if !self.composer.focus(cx).is_focused(window) {
                window.focus(self.menus.usage_focus(), cx);
            }
            cx.notify();
        }
    }
    pub(super) fn usage_report(&self, cx: &App) -> Option<Report> {
        let thread = self.current()?;
        let thread = thread.read(cx);
        let context = thread
            .transcript()
            .usage
            .clone()
            .filter(|usage| usage.size > 0);
        let breakdown = context
            .as_ref()
            .map(|usage| Breakdown::estimate(&thread.transcript().entries, usage.used))
            .unwrap_or_default();
        let limits: Vec<RateLimit> = self
            .host
            .runtime()
            .limits(thread.document().agent_id(), cx)
            .map(|limits| limits.iter().cloned().collect())
            .unwrap_or_default();
        let limits_hint = self
            .host
            .agents()
            .profile(thread.document().agent_id(), cx)
            .and_then(|profile| profile.usage().empty_hint())
            .filter(|_| limits.is_empty());
        Some(Report::new(
            context,
            breakdown,
            thread.transcript().tokens.clone(),
            limits,
            limits_hint,
        ))
    }
    pub(super) fn render_usage(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(report) = self.usage_report(cx) else {
            return div().into_any_element();
        };
        usage::UsageCard::render(&report, self.menus.usage_focus(), self.host.timestamp(), cx)
    }
}

impl AgentPanel {
    pub(super) fn on_usage_dismiss_requested(&mut self, cx: &mut Context<Self>) {
        self.dismiss_usage(cx);
    }
    pub(super) fn on_usage_focus_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menus.usage_contains_focus(window, cx) {
            self.dismiss_usage(cx);
        }
    }
}
