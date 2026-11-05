use super::transcript::TranscriptView;
use super::usage::Report;
use super::*;
use super::{
    entries::{EntryRenderer, RowPresentation},
    widgets::entry_is_live,
};
use gpui_kit::AnyElement;
use nocterm_agent_chat::conversation::{ConversationError, ConversationSource, LocalConversation};
use nocterm_ai::conversation::PendingPromptKey;
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
    /// Sends a failed pending prompt of the shown conversation again. While the composer still
    /// holds it, this is the composer's own send: a refused prompt becomes a new one and an
    /// unanswered one keeps its ID. Otherwise its source retries it.
    pub(super) fn retry_pending_prompt(
        &mut self,
        identity: EntityId,
        prompt: PendingPromptKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .conversation
            .as_ref()
            .filter(|selected| selected.identity() == identity)
        else {
            return;
        };
        let composing = self
            .composer
            .input_ticket(cx)
            .ok()
            .and_then(|ticket| selected.pending(&ticket))
            .is_some_and(|(submission, _)| submission.pending_prompt() == Some(prompt));
        if composing {
            self.submit_conversation(false, window, cx);
            return;
        }
        let work = match selected.source().capture_actions(cx) {
            Ok(actions) => actions.retry_prompt(prompt, cx),
            Err(error) => gpui_kit::Task::ready(Err(error)),
        };
        self.retain_conversation_action(identity, work, None, cx);
    }
    /// Forgets a refused prompt of the shown conversation.
    pub(super) fn dismiss_pending_prompt(
        &mut self,
        identity: EntityId,
        prompt: PendingPromptKey,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self
            .conversation
            .as_ref()
            .filter(|selected| selected.identity() == identity)
        else {
            return;
        };
        let dismissed = selected
            .source()
            .capture_actions(cx)
            .and_then(|actions| actions.dismiss_prompt(prompt, cx));
        if let Err(error) = dismissed {
            self.notices.set_error(Some(error.to_string()));
        }
        cx.notify();
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
    /// The open chat's or conversation's footer: approvals, status, queue, commands and the
    /// composer. Every source draws the same footer; its capabilities decide what it offers.
    /// A Local chat adds what its thread owns: its status, queue and requests.
    pub(super) fn render_footer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let local = self.current();
        let source: Rc<dyn ConversationSource> = match (&self.conversation, &local) {
            (Some(selected), _) => selected.source(),
            (None, Some(thread)) => Rc::new(LocalConversation::new(thread)),
            (None, None) => return div().into_any_element(),
        };
        // The transcript above names why a source cannot be read.
        let Some(frame) = super::composer_frame::ComposerFrame::read(source.as_ref(), cx) else {
            return div().into_any_element();
        };
        let busy = match (&self.conversation, self.composer.input_ticket(cx)) {
            (Some(selected), Ok(ticket)) => selected.pending(&ticket).is_some_and(|(_, busy)| busy),
            _ => false,
        };
        let approvals = self.render_approvals(source, local.as_ref(), window, cx);
        let status = match &local {
            Some(thread) => self.chat_status(thread, cx),
            None => frame
                .status()
                .map(|(text, live)| TranscriptView::status(frame.origin(), text, live, cx)),
        };
        let wake = (local.is_none() && frame.can_wake())
            .then(|| TranscriptView::wake_button(frame.origin(), cx));
        let servers = self
            .conversation
            .as_ref()
            .and_then(|selected| selected.servers());
        let labels = local
            .as_ref()
            .map(|thread| attachments::AttachmentLabels::new(thread.read(cx).workspace_seat(), cx));
        let queue = match (&local, &labels) {
            (Some(thread), Some(labels)) => self.render_queue(thread, labels, window, cx),
            _ => None,
        };
        let commands = self.render_commands(window, cx);
        let composer = self.render_composer(&frame, local.as_ref(), labels.as_ref(), busy, cx);
        v_flex()
            .w_full()
            .min_w_0()
            .children(approvals)
            .children(status)
            .children(wake)
            .children(servers)
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
    /// The cards of every pending request: the source's tool approvals and, of a Local chat,
    /// its sign-in and permission requests; none when nothing waits.
    pub(super) fn render_approvals(
        &mut self,
        source: Rc<dyn ConversationSource>,
        local: Option<&Entity<AgentThread>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let mut list = Vec::new();
        if let Some(thread) = local {
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
        let tools = source.tool_approvals(cx).unwrap_or_default();
        for (index, tool) in tools.into_iter().enumerate() {
            if let Some(card) =
                approvals::ApprovalCardsRenderer::tool_card(tool, source.identity(), index, cx)
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
    /// The transcript of the open chat or conversation, for its cached view; the
    /// rows changed since its last drawing redraw.
    pub(super) fn render_transcript_part(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (source, dirty): (Rc<dyn ConversationSource>, Vec<usize>) =
            if let Some(selected) = self.conversation.as_mut() {
                let source = selected.source();
                let mut dirty = Vec::new();
                if selected.take_dirty() {
                    let _ = source.with_view(cx, &mut |view| {
                        dirty.extend(0..view.transcript.entries.len() + view.pending_prompts.len());
                    });
                }
                (source, dirty)
            } else if let Some(thread) = self.current() {
                let dirty = thread.update(cx, |thread, _| thread.take_changed_rows());
                (
                    Rc::new(LocalConversation::new(&thread)),
                    dirty.into_iter().collect(),
                )
            } else {
                return div().into_any_element();
            };
        let mut stamp = None;
        let _ = source.with_view(cx, &mut |view| stamp = Some(parts::transcript_stamp(&view)));
        self.layout.parts_mut().drew_transcript(stamp);
        match self.render_transcript(source, &dirty, cx) {
            Ok(body) => body,
            Err(error) => div().child(error.to_string()).into_any_element(),
        }
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
            .size_full()
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
        let mut markdown = None;
        let _ = source.with_view(cx, &mut |view| {
            markdown = view.transcript.entries.get(index).and_then(|entry| {
                super::message_markdown::MessageMarkdown::text(entry, index, &self.messages)
            });
        });
        if let Some((text, source_len)) = markdown {
            self.messages.prepare_markdown(index, text, source_len, cx);
        }
        let mut rendered = None;
        let current = source.with_view(cx, &mut |view| {
            let pending = super::pending_prompt::PendingMark::at(&view, index);
            let Some(entry) = view
                .transcript
                .entries
                .get(index)
                .or(pending.as_ref().map(|(_, entry)| entry))
            else {
                return;
            };
            let mut frame = RowPresentation::new(
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
            if let Some((mark, _)) = &pending {
                frame = frame.with_pending(*mark);
            }
            rendered = Some(EntryRenderer::render(frame, entry, &self.messages, cx));
        });
        let Some((row, pending)) = current.ok().and(rendered) else {
            return div().into_any_element();
        };
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
                view.pending_prompts.len(),
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
