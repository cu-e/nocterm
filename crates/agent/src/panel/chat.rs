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
    pub(super) fn sync_approval_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let attention = self
            .chats
            .iter()
            .any(|thread| thread.read(cx).has_pending_approvals());
        let needs_notice = attention
            && (self
                .current()
                .is_none_or(|thread| !thread.read(cx).has_pending_approvals())
                || self
                    .host
                    .workspace()
                    .upgrade()
                    .is_none_or(|workspace| !workspace.read(cx).right_panel_is_open()));
        let _ = self.host.workspace().update(cx, |workspace, cx| {
            workspace.set_right_panel_attention(attention, cx)
        });
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
                    if !workspace.right_panel_is_open() {
                        workspace.toggle_right_panel(window, cx);
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
    /// The cards of every pending request of `thread`; none when nothing waits.
    pub(super) fn render_approvals(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let permissions = thread.read(cx).permission_cards();
        let tools = thread.read(cx).tool_cards();
        let pending = permissions.len() + tools.len() + thread.read(cx).sign_in_waits().len();
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
        if pending == 0 {
            return None;
        }
        let mut list = v_flex()
            .id("agent-approval-list")
            .test_support()
            .w_full()
            .min_w_0()
            .min_h_0()
            .max_h(window.viewport_size().height * 0.35)
            .overflow_y_scroll()
            .gap_2();
        for index in 0..thread.read(cx).sign_in_waits().len() {
            let wait = thread.read(cx).sign_in_waits()[index].clone();
            let (input, submission) = self.sign_ins.input_for(thread, &wait, window, cx);
            list = list.child(approvals::ApprovalCardsRenderer::sign_in_card(
                wait, input, submission, index, cx,
            ));
        }
        for (index, permission) in permissions.into_iter().enumerate() {
            list = list.child(approvals::ApprovalCardsRenderer::permission_card(
                thread, permission, index, cx,
            ));
        }
        for (index, tool) in tools.into_iter().enumerate() {
            let call = tool.call();
            let target = thread.update(cx, |thread, cx| thread.describe_target(call, cx));
            let reusable = thread.update(cx, |thread, cx| !thread.unsafe_live_input(call, cx));
            list = list.child(approvals::ApprovalCardsRenderer::tool_card(
                thread, tool, target, reusable, index, cx,
            ));
        }
        Some(
            v_flex()
                .id("agent-approvals")
                .test_support()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .px_3()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(format!("Waiting for you · {pending}")),
                )
                .child(list)
                .into_any_element(),
        )
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
        let Some(thread) = self.current() else {
            return;
        };
        let reduced = cx.reduce_motion();
        let changed =
            self.messages
                .advance_reveal(&binding, thread.read(cx).history_entries(), reduced);
        if let Some(changed) = changed {
            self.messages.apply_reveal(changed, cx);
            cx.notify();
        }
    }
    pub(super) fn on_preview_completed(
        &mut self,
        binding: message_list::ActiveListBinding,
        ticket: image_pipeline::PreviewTicket,
        row: Option<usize>,
        image: Option<std::sync::Arc<gpui_kit::Image>>,
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
    pub(super) fn render_chat(
        &mut self,
        thread: Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = v_flex().size_full().min_w_0().min_h_0();
        let plan = thread.read(cx).transcript().plan.clone();
        let usage = thread.read(cx).transcript().usage.clone();
        let dirty = thread
            .update(cx, |thread, _| thread.take_changed_rows())
            .into_iter()
            .collect::<Vec<_>>();
        self.messages.bind_chat(thread.entity_id(), cx);
        let rows = self.messages.observe_rows(
            thread.entity_id(),
            thread.read(cx).history_entries(),
            &dirty,
            thread.read(cx).session_generating(),
            cx.reduce_motion(),
        );
        self.messages.apply_rows(rows, cx);
        let scroller = self.messages.scroller(cx);
        let navigation = navigation::NavigationRenderer::render(&self.messages, cx);
        let overlay = self
            .menus
            .is_open(&MenuKind::Usage)
            .then(|| self.render_usage(cx));
        body = body.child(TranscriptView::area(scroller, navigation, overlay, cx));
        if let Some(plan) = &plan {
            body = body.child(TranscriptView::plan(plan, cx));
        }
        if let Some(approvals) = self.render_approvals(&thread, window, cx) {
            body = body.child(approvals);
        }
        let labels = attachments::AttachmentLabels::new(thread.read(cx).workspace_seat(), cx);
        let composer = self.render_composer(&thread, usage.as_ref(), &labels, cx);
        let status = self.chat_status(&thread, cx);
        body = body
            .children(status)
            .children(self.render_queue(&thread, &labels, window, cx))
            .children(self.render_commands(window, cx))
            .child(composer);
        body.into_any_element()
    }
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(thread) = self.current() else {
            return div().into_any_element();
        };
        let state = thread.read(cx);
        let Some(entry) = state.history_entries().get(index) else {
            return div().into_any_element();
        };
        let frame = RowPresentation::new(
            thread.entity_id(),
            self.messages.binding(),
            index,
            self.messages.expanded(index),
            entry_is_live(
                state.history_entries(),
                index,
                state.session_generating() && state.session_accepts_updates(),
            ),
            state.session_generating(),
            state.transcript().time(index),
            self.settings
                .setting::<nocterm_ai::AiSettings>(cx)
                .approval
                .redact_secrets,
        );
        let (row, pending) = EntryRenderer::render(frame, entry, &self.messages, cx);
        for (key, data) in pending {
            preview_operation::PreviewOperation::cached(
                &mut self.messages,
                key,
                || data,
                true,
                Some(index),
                cx,
            );
        }
        row
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
            self.refresh_usage(cx);
            self.menus.open(MenuKind::Usage);
            // Read-only information must not interrupt composing.
            if !self.composer.focus(cx).is_focused(window) {
                window.focus(self.menus.usage_focus(), cx);
            }
            cx.notify();
        }
    }
    pub(super) fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        if let Some(thread) = self.current() {
            let agent = thread.read(cx).document().agent_id().to_owned();
            self.host.runtime().refresh_limits(&agent, cx);
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

impl AgentPanel {
    pub(super) fn render_queue(
        &mut self,
        thread: &Entity<AgentThread>,
        labels: &attachments::AttachmentLabels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if thread.read(cx).composer().queue().is_empty() {
            return None;
        }
        let height = queue::QueueRenderer::visible_height(&self.composer, thread, window, cx);
        let prompts = if self.composer.queue_open() || height > gpui_kit::px(0.) {
            thread.read(cx).composer().queue().to_vec()
        } else {
            Vec::new()
        };
        let previews =
            preview_operation::PreviewOperation::queued_previews(&mut self.messages, &prompts, cx);
        queue::QueueRenderer::render_queue(
            &self.composer,
            prompts,
            previews,
            thread,
            labels,
            height,
            cx,
        )
    }
}
