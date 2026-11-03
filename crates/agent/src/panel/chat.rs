use super::transcript::TranscriptView;
use super::usage::Report;
use super::*;
use super::{
    entries::{EntryRenderer, RowPresentation},
    widgets::entry_is_live,
};
use gpui_kit::AnyElement;
use nocterm_agent_chat::conversation::{ConversationError, ConversationSource, LocalConversation};
use nocterm_ai::usage::{Breakdown, RateLimit};
use std::rc::Rc;

impl AgentPanel {
    pub(crate) fn open_conversation(
        &mut self,
        source: Rc<dyn ConversationSource>,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        source.with_view(cx, &mut |_| {})?;
        if self
            .conversation
            .as_ref()
            .is_some_and(|view| view.identity() == source.identity())
        {
            return Ok(());
        }
        let previous_local = self.current().map(|thread| thread.entity_id()).or_else(|| {
            self.conversation
                .as_ref()
                .and_then(|view| view.previous_local())
        });
        let selected = super::conversation_surface::ConversationSurface::attach(
            source,
            title,
            previous_local,
            cx.weak_entity(),
            cx,
        )?;
        self.leave_composer(cx);
        self.chats.deselect(cx);
        if let Some(previous) = self.conversation.replace(selected) {
            previous.detach(cx);
        }
        self.reset_chat_view(cx);
        self.composer.clear();
        self.composer.set_text("", window, cx);
        self.notices.set_error(None);
        cx.notify();
        Ok(())
    }
    pub(crate) fn close_conversation(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.conversation.take() {
            previous.detach(cx);
            self.reset_chat_view(cx);
            self.composer.clear();
            if let Some(local) = previous
                .previous_local()
                .filter(|id| self.chats.contains(*id))
            {
                self.chats.select(local, cx);
                self.load_composer(cx);
            }
            cx.notify();
        }
    }
    pub(super) fn on_conversation_changed(&mut self, identity: EntityId, cx: &mut Context<Self>) {
        if let Some(selected) = self
            .conversation
            .as_mut()
            .filter(|view| view.identity() == identity)
        {
            selected.changed();
            cx.notify();
        }
    }
    pub(super) fn submit_conversation(
        &mut self,
        stop_when_empty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use nocterm_ai::conversation::ConversationAction;
        let text = self.composer.text(cx).to_string();
        let action = if text.trim().is_empty() {
            if !stop_when_empty {
                return;
            }
            ConversationAction::Stop
        } else {
            ConversationAction::Prompt(text)
        };
        self.perform_conversation_action(action, Some(window), cx);
    }
    pub(super) fn perform_conversation_action(
        &mut self,
        action: nocterm_ai::conversation::ConversationAction,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let input = match self.composer.input_ticket(cx) {
            Ok(input) => input,
            Err(error) => {
                self.notices.set_error(Some(error.to_string()));
                cx.notify();
                return;
            }
        };
        let Some(selected) = self.conversation.as_mut() else {
            return;
        };
        let identity = selected.identity();
        let clear = matches!(
            &action,
            nocterm_ai::conversation::ConversationAction::Prompt(_)
        )
        .then(|| window.map(|window| (window.window_handle(), input.clone())))
        .flatten();
        let work = match selected.submit(action, input, cx) {
            Ok(Some(work)) => work,
            Ok(None) => return,
            Err(error) => {
                self.notices.set_error(Some(error.to_string()));
                cx.notify();
                return;
            }
        };
        self.retain_conversation_action(identity, work, clear, cx);
    }
    /// Sends the person's permission for the displayed run, if it is still the shown source.
    pub(super) fn allow_conversation_run(&mut self, identity: EntityId, cx: &mut Context<Self>) {
        if self.conversation_identity() != Some(identity) {
            return;
        }
        self.perform_conversation_action(
            nocterm_ai::conversation::ConversationAction::Allow,
            None,
            cx,
        );
    }
    pub(super) fn perform_rendered_stop(
        &mut self,
        identity: EntityId,
        actions: Rc<dyn nocterm_agent_chat::conversation::ConversationActions>,
        input: super::composer_view::InputTicket,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_identity() != Some(identity)
            || !self.composer.input_current(&input, cx)
        {
            return;
        }
        let work = actions.perform(nocterm_ai::conversation::ConversationAction::Stop, cx);
        self.retain_conversation_action(identity, work, None, cx);
    }
    pub(super) fn capture_rendered_stop(
        &self,
        cx: &App,
    ) -> Option<Rc<super::conversation_surface::RenderedStop>> {
        let source: Rc<dyn ConversationSource> = match &self.conversation {
            Some(selected) => selected.source(),
            None => Rc::new(LocalConversation::new(&self.current()?)),
        };
        let mut can_stop = false;
        source
            .with_view(cx, &mut |view| can_stop = view.can_stop)
            .ok()?;
        if !can_stop || !self.composer.text(cx).trim().is_empty() {
            return None;
        }
        super::conversation_surface::RenderedStop::capture(
            source,
            self.composer.input_ticket(cx).ok()?,
            cx,
        )
        .ok()
    }
    pub(super) fn choose_conversation_tool(
        &mut self,
        identity: EntityId,
        approval: Rc<dyn nocterm_agent_chat::conversation::ConversationToolApproval>,
        choice: nocterm_agent_chat::conversation::ToolApprovalChoice,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_identity() != Some(identity) {
            return;
        }
        let work = approval.choose(choice, cx);
        self.retain_conversation_action(identity, work, None, cx);
    }
    fn retain_conversation_action(
        &mut self,
        identity: EntityId,
        work: gpui_kit::Task<Result<(), ConversationError>>,
        clear: Option<(gpui_kit::AnyWindowHandle, super::composer_view::InputTicket)>,
        cx: &mut Context<Self>,
    ) {
        let ticket = self.composer.input_ticket(cx).ok();
        cx.spawn(async move |panel, cx| {
            let result = work.await;
            let _ = panel.update(cx, |panel, cx| {
                if panel.conversation_identity() != Some(identity) {
                    return;
                }
                let Some(ticket) = &ticket else {
                    return;
                };
                if !panel.composer.input_current(ticket, cx) {
                    return;
                }
                if clear.is_some()
                    && let Some(selected) = &mut panel.conversation
                {
                    selected.completed(ticket, result.is_err());
                }
                match result {
                    Err(error) => panel.notices.set_error(Some(error.to_string())),
                    Ok(()) => {
                        panel.notices.set_error(None);
                        if let Some((window, input)) = clear {
                            let weak = cx.weak_entity();
                            cx.defer(move |cx| {
                                let _ = window.update(cx, |_, window, cx| {
                                    let _ = weak.update(cx, |panel, cx| {
                                        if panel.conversation_identity() == Some(identity)
                                            && panel.composer.input_current(&input, cx)
                                        {
                                            panel.composer.set_text("", window, cx);
                                        }
                                    });
                                });
                            });
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(selected) = self.conversation.as_mut() else {
            return div().into_any_element();
        };
        let source = selected.source();
        let busy = self
            .composer
            .input_ticket(cx)
            .ok()
            .and_then(|ticket| selected.pending(&ticket))
            .is_some_and(|(_, busy)| busy);
        let mut dirty = Vec::new();
        let changed = selected.take_dirty();
        let mut status = None;
        if let Err(error) = source.with_view(cx, &mut |view| {
            status = Some(TranscriptView::conversation_status(&view));
            if changed {
                dirty.extend(0..view.transcript.entries.len());
            }
        }) {
            return div().child(error.to_string()).into_any_element();
        }
        let body = match self.render_transcript(source.clone(), &dirty, cx) {
            Ok(body) => body,
            Err(error) => return div().child(error.to_string()).into_any_element(),
        };
        let approvals = self.render_source_approvals(source.clone(), window, cx);
        let stop = self.capture_rendered_stop(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(body)
            .children(approvals)
            .children(status.map(|text| TranscriptView::status(source.identity(), text, false, cx)))
            .child(composer_renderer::ComposerRenderer::render_conversation(
                &self.composer,
                source,
                busy,
                stop,
                cx,
            ))
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
    /// The cards of every pending request of `thread`; none when nothing waits.
    pub(super) fn render_approvals(
        &mut self,
        thread: &Entity<AgentThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let permissions = thread.read(cx).permission_cards();
        let tools = LocalConversation::new(thread)
            .tool_approvals(cx)
            .unwrap_or_default();
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
        let mut list = Vec::new();
        for index in 0..thread.read(cx).sign_in_waits().len() {
            let wait = thread.read(cx).sign_in_waits()[index].clone();
            let (input, submission) = self.sign_ins.input_for(thread, &wait, window, cx);
            list.push(approvals::ApprovalCardsRenderer::sign_in_card(
                wait, input, submission, index, cx,
            ));
        }
        for (index, permission) in permissions.into_iter().enumerate() {
            list.push(approvals::ApprovalCardsRenderer::permission_card(
                thread, permission, index, cx,
            ));
        }
        for (index, tool) in tools.into_iter().enumerate() {
            if let Some(card) =
                approvals::ApprovalCardsRenderer::tool_card(tool, thread.entity_id(), index, cx)
            {
                list.push(card);
            }
        }
        approvals::ApprovalCardsRenderer::list(list, window, cx)
    }
    fn render_source_approvals(
        &mut self,
        source: Rc<dyn ConversationSource>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let cards = source
            .tool_approvals(cx)
            .ok()?
            .into_iter()
            .enumerate()
            .filter_map(|(index, card)| {
                approvals::ApprovalCardsRenderer::tool_card(card, source.identity(), index, cx)
            })
            .collect();
        approvals::ApprovalCardsRenderer::list(cards, window, cx)
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
        let Some((_, source)) = self.messages.source() else {
            return;
        };
        let reduced = cx.reduce_motion();
        let mut changed = None;
        let _ = source.with_view(cx, &mut |view| {
            changed = self
                .messages
                .advance_reveal(&binding, &view.transcript.entries, reduced);
        });
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
        let usage = thread.read(cx).transcript().usage.clone();
        let dirty = thread
            .update(cx, |thread, _| thread.take_changed_rows())
            .into_iter()
            .collect::<Vec<_>>();
        let transcript =
            match self.render_transcript(Rc::new(LocalConversation::new(&thread)), &dirty, cx) {
                Ok(body) => body,
                Err(error) => return div().child(error.to_string()).into_any_element(),
            };
        let mut body = v_flex().size_full().min_w_0().min_h_0().child(transcript);
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
    fn render_transcript(
        &mut self,
        source: Rc<dyn ConversationSource>,
        dirty: &[usize],
        cx: &mut Context<Self>,
    ) -> Result<AnyElement, ConversationError> {
        self.observe_source_rows(source.clone(), source.identity(), dirty, cx)?;
        let scroller = self.messages.scroller(cx);
        let navigation = navigation::NavigationRenderer::render(&self.messages, cx);
        let overlay = self
            .menus
            .is_open(&MenuKind::Usage)
            .then(|| self.render_usage(cx));
        let mut plan = None;
        source.with_view(cx, &mut |view| {
            plan = view
                .transcript
                .plan
                .as_ref()
                .map(|plan| TranscriptView::plan(plan, cx));
        })?;
        let body = v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(TranscriptView::area(scroller, navigation, overlay, cx));
        Ok(body.children(plan).into_any_element())
    }
    pub(super) fn render_entry(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some((identity, source)) = self.messages.source() else {
            return div().into_any_element();
        };
        let mut rendered = None;
        let current = source.with_view(cx, &mut |view| {
            let Some(entry) = view.transcript.entries.get(index) else {
                return;
            };
            let frame = RowPresentation::new(
                identity,
                self.messages.binding(),
                index,
                self.messages.expanded(index),
                entry_is_live(
                    &view.transcript.entries,
                    index,
                    view.generating && view.accepts_updates,
                ),
                view.generating,
                view.transcript.time(index),
                self.settings
                    .setting::<nocterm_ai::AiSettings>(cx)
                    .approval
                    .redact_secrets,
            )
            .with_fork_available(view.can_fork);
            rendered = Some(EntryRenderer::render(frame, entry, &self.messages, cx));
        });
        let Some((row, pending)) = current.ok().and(rendered) else {
            return div().into_any_element();
        };
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
    /// The visible list retains its original source. Each observer supplies its
    /// own dirty rows, so one window never consumes another's change tracking.
    pub(super) fn observe_source_rows(
        &mut self,
        source: Rc<dyn ConversationSource>,
        identity: EntityId,
        dirty: &[usize],
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        self.messages.bind_source(source, identity, cx);
        let (_, source) = self.messages.source().ok_or(ConversationError::Stale)?;
        let reduced = cx.reduce_motion();
        let mut rows = None;
        source.with_view(cx, &mut |view| {
            rows = Some(self.messages.observe_rows(
                identity,
                &view.transcript.entries,
                dirty,
                view.generating,
                reduced,
            ));
        })?;
        if let Some(rows) = rows {
            self.messages.apply_rows(rows, cx);
        }
        Ok(())
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
