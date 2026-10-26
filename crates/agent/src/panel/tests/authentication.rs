//! Sign-in controls, prompt authentication and terminal sign-in retries.
use super::*;
use nocterm_ai::model::tools::ToolStatus;

#[gpui_kit::test]
fn authentication_controls_follow_required_state_and_create_session_after_click(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).session_sign_in_required());
        assert!(
            thread.read(cx).registration().is_some(),
            "auth-required must preserve bridge for retry"
        );
        assert!(thread.read(cx).session().is_none());
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).session_sign_in_required());
        assert!(thread.read(cx).session().is_some());
        assert_eq!(thread.read(cx).status(), "Ready");
        assert!(window.try_find("authenticate-sign-in").is_none());
        assert!(window.try_find("agent-status").is_none());
    })
    .unwrap();
    assert_eq!(f.commands.authentications.load(Ordering::SeqCst), 1);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 2);
}

#[gpui_kit::test]
fn ready_hides_auth_and_status_while_working_status_precedes_composer(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("authenticate-sign-in").is_none());
        assert!(window.try_find("agent-status").is_none());
    })
    .unwrap();
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| {
                thread.send("Question".into(), cx);
            });
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let status = window.find("agent-status").bounds();
        let composer = window.find("agent-composer").bounds();
        assert!(status.bottom() <= composer.origin.y);
        let thinking = window.find("agent-thinking").bounds();
        assert!(status.contains(&thinking.origin));
        assert_eq!(thinking.origin.x, status.origin.x + gpui_kit::px(12.));
        let thread = f.panel.read(cx).current().unwrap();
        assert_eq!(thread.read(cx).status(), "Thinking…");
        assert!(window.try_find("authenticate-sign-in").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn prompt_authentication_failure_keeps_retry_and_existing_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    f.commands
        .authentication_failure
        .store(true, Ordering::SeqCst);
    cx.update(|cx| {
        f.panel
            .read(cx)
            .current()
            .unwrap()
            .update(cx, |thread, cx| { let entries = ["pending", "completed"].into_iter().map(|status| {
                    nocterm_ai::thread::Entry::Tool(serde_json::from_value(serde_json::json!({"toolCallId":status,"title":"Before auth required","status":status})).unwrap())
                }).collect();
                restore_entries(&f, thread, entries, cx);
                thread.send("Question".into(), cx); });
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).session_sign_in_required());
        assert!(!thread.read(cx).session_generating());
        let statuses = thread
            .read(cx)
            .transcript()
            .entries
            .iter()
            .filter_map(|entry| {
                if let nocterm_ai::thread::Entry::Tool(call) = entry {
                    Some(call.status)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(statuses, vec![ToolStatus::Failed, ToolStatus::Succeeded]);
        thread.update(cx, |thread, cx| {
            thread.send("Do not submit while auth-required".into(), cx)
        });
        assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).session_sign_in_required());
        assert!(!thread.read(cx).session_signing_in());
        assert_eq!(thread.read(cx).status(), "Sign-in failed");
        assert!(thread.read(cx).session().is_some());
        f.commands
            .authentication_failure
            .store(false, Ordering::SeqCst);
        window.click("authenticate-sign-in", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).session_sign_in_required());
        assert!(thread.read(cx).session().is_some());
        assert!(window.try_find("authenticate-sign-in").is_none());
    })
    .unwrap();
    assert_eq!(f.commands.authentications.load(Ordering::SeqCst), 2);
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), 1);
}

/// Asserts that terminal sign-in failed with `status` and remains retryable, then retries it.
fn retry_failed_terminal_sign_in(f: &Fixture, status: &str, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(thread.read(cx).session_sign_in_required());
        assert!(!thread.read(cx).session_signing_in());
        assert_eq!(thread.read(cx).status(), status);
        window.click("authenticate-setup", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn terminal_authentication_uses_host_callback_and_retries_after_cancel_or_failure(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    f.commands.auth_required.store(true, Ordering::SeqCst);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let commands = f.commands.clone();
    let opener: crate::TerminalAuthOpener = Arc::new(move |_, request, _, _| {
        let mut requests = captured.lock().unwrap();
        requests.push(request);
        let (send, receive) = oneshot::channel();
        match requests.len() {
            1 => {}
            2 => send.send(Err(crate::TerminalAuthError::Failed)).unwrap(),
            _ => {
                commands.auth_required.store(false, Ordering::SeqCst);
                send.send(Ok(())).unwrap();
            }
        }
        receive
    });
    install_fixture(
        &mut f,
        |f| {
            let mut services = fixture_services(f);
            services.terminal_auth = Some(opener);
            services.usage_source = Arc::new(nocterm_ai::testing::ScriptedUsageSource::default());
            services
        },
        cx,
    );
    let unrelated = fixture(cx);
    f.connector.report_auth_methods(vec![AuthMethod::terminal(
        "setup",
        "Terminal setup",
        nocterm_ai::model::auth::TerminalSetup {
            args: vec!["--setup".into(), "$(literal_argument)".into()],
            env: Default::default(),
        },
    )]);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("authenticate-setup", cx);
    })
    .unwrap();
    cx.run_until_parked();
    retry_failed_terminal_sign_in(&f, "Sign-in cancelled", cx);
    retry_failed_terminal_sign_in(
        &f,
        "Authentication command did not complete successfully.",
        cx,
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let thread = f.panel.read(cx).current().unwrap();
        assert!(!thread.read(cx).session_sign_in_required());
        assert!(thread.read(cx).session().is_some());
        assert!(window.try_find("authenticate-setup").is_none());
    })
    .unwrap();
    assert_eq!(
        f.commands.authentications.load(Ordering::SeqCst),
        0,
        "terminal methods must not use the agent authenticate RPC"
    );
    assert_eq!(unrelated.connector.connects.load(Ordering::SeqCst), 0);
    assert_eq!(unrelated.commands.authentications.load(Ordering::SeqCst), 0);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].program, "npx");
    assert!(
        requests[0]
            .args
            .ends_with(&["--setup".into(), "$(literal_argument)".into()])
    );
}
