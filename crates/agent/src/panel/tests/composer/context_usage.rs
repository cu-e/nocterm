//! Existing context and plan usage presentation.
use super::*;

#[gpui_kit::test]
fn usage_card_shows_context_tokens_and_plan_limits(cx: &mut TestAppContext) {
    let f = fixture(cx);
    panel_actions::new_agent_chat(&f, "claude", cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    for (kind, used) in [("seven_day", 0.38), ("five_hour", 0.12)] {
        f.events
            .try_send(AgentEvent::Session(SessionNotice::new(
                session.clone(),
                SessionUpdate::UsageUpdate(plan_usage(kind, used)),
            )))
            .unwrap();
    }
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(PromptOutcome {
            usage: Some(TokenTotals {
                cache_read: Some(20_000),
                ..TokenTotals::new(30_000, 25_000, 5_000)
            }),
            ..PromptOutcome::new(StopReason::EndTurn)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let runtime = f.installation.installed.runtime();
        let limits: Vec<_> = runtime
            .limits("claude", cx)
            .unwrap()
            .iter()
            .cloned()
            .collect();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].window, nocterm_ai::usage::Window::FiveHour);
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(
            thread.read(cx).transcript().tokens.as_ref().unwrap().total,
            30_000
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        let card = window.find("agent-usage").bounds();
        let composer = window.find("agent-composer").bounds();
        // Over the chat, above the composer.
        assert!(card.bottom() <= composer.origin.y + gpui_kit::px(1.));
        window.click("agent-usage-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn context_tokens_and_cost_remain_available_without_plan_limits(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
    cx.run_until_parked();
    let session = thread.read_with(cx, |thread, _| thread.session().clone().unwrap());
    let mut usage = ContextUsage::new(50, 100);
    usage.cost = Some(nocterm_ai::model::usage::Cost {
        amount: 1.25,
        currency: "USD".into(),
    });
    f.events
        .try_send(AgentEvent::Session(SessionNotice::new(
            session,
            SessionUpdate::UsageUpdate(usage.clone()),
        )))
        .unwrap();
    cx.run_until_parked();
    f.commands
        .pending
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .send(PromptOutcome {
            usage: Some(TokenTotals::new(30, 20, 10)),
            ..PromptOutcome::new(StopReason::EndTurn)
        })
        .unwrap();
    cx.run_until_parked();
    thread.read_with(cx, |thread, _| {
        assert_eq!(thread.transcript().usage, Some(usage));
        assert_eq!(
            thread.transcript().tokens,
            Some(TokenTotals::new(30, 20, 10))
        );
    });
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-context-usage", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-usage").is_some());
        assert!(
            f.installation
                .installed
                .runtime()
                .limits("codex", cx)
                .is_none()
        );
    })
    .unwrap();
}

fn plan_usage(kind: &str, used: f64) -> ContextUsage {
    let meta = serde_json::json!({"_claude/rateLimit": {
        "rateLimitType": kind,
        "utilization": used,
        "resetsAt": 1_791_588_105u64,
        "status": "allowed"
    }});
    ContextUsage::new(25_900, 256_000).meta(
        meta.as_object()
            .cloned()
            .map(nocterm_ai::model::extensions::Extensions::from),
    )
}

#[gpui_kit::test]
fn received_plan_limits_stay_with_their_original_installation_after_prompt_completion(
    cx: &mut TestAppContext,
) {
    let first = fixture(cx);
    panel_actions::new_agent_chat(&first, "claude", cx);
    let thread = cx.update(|cx| first.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| thread.send("original prompt".into(), cx));
    cx.run_until_parked();
    let finish = first.commands.pending.lock().unwrap().take().unwrap();
    let second = fixture(cx);
    panel_actions::new_agent_chat(&second, "claude", cx);
    let session = thread.read_with(cx, |thread, _| thread.session().clone().unwrap());
    first
        .events
        .try_send(AgentEvent::Session(SessionNotice::new(
            session,
            SessionUpdate::UsageUpdate(plan_usage("seven_day", 0.38)),
        )))
        .unwrap();
    finish
        .send(PromptOutcome::new(StopReason::EndTurn))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let limits = first
            .installation
            .installed
            .runtime()
            .limits("claude", cx)
            .unwrap();
        assert_eq!(limits.iter().next().unwrap().used, Some(0.38));
        assert!(
            second
                .installation
                .installed
                .runtime()
                .limits("claude", cx)
                .is_none()
        );
    });
}
