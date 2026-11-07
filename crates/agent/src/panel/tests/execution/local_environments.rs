//! Original ACP callbacks cannot consume credentials belonging to a later account.
use super::*;
use nocterm_hubs_model::{
    source_identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition, NamespaceId,
    },
    testing::ScriptedLocalAccountFactory,
};
fn account(name: &str) -> LocalAccountPartition {
    LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("hub").unwrap(),
            Fingerprint::from_bytes([1; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new(name).unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    ))
}
fn account_fixture(factory: Arc<ScriptedLocalAccountFactory>, cx: &mut TestAppContext) -> Fixture {
    let mut f = fixture(cx);
    let installed = f
        .installation
        .installed
        .with_account_source(factory.clone());
    if let Ok(runtime) = installed.capture_runtime() {
        f.access.bind_source(runtime.captured_source().unwrap());
    }
    let (handle, workspace, panel, terminal) = cx.update(|cx| {
        open_host(
            &f._directory,
            &installed,
            &f.installation.layout,
            &f.access,
            cx,
        )
    });
    f.installation.installed = installed;
    f.handle = handle;
    f.workspace = workspace;
    f.panel = panel;
    f.terminal = terminal;
    f
}
#[gpui_kit::test]
fn locked_account_host_can_capture_after_unlock_without_rebuilding_its_window(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    let f = account_fixture(factory.clone(), cx);
    cx.update(|cx| assert!(f.panel.read(cx).current().is_none()));
    assert!(f.installation.installed.capture_runtime().is_err());
    factory.sign_in(account("alice"));
    cx.update(|cx| {
        f.installation
            .installed
            .runtime()
            .refresh_environment_sources(cx)
    });
    cx.run_until_parked();
    new_chat(&f, cx);
    let original = current(&f, cx);
    let original_key = cx.update(|cx| original.read(cx).document().key().clone());
    assert!(!original_key.environment().is_this_computer());
    factory.sign_in(account("bob"));
    cx.update(|cx| {
        f.installation
            .installed
            .runtime()
            .refresh_environment_sources(cx)
    });
    cx.run_until_parked();
    new_chat(&f, cx);
    let replacement = current(&f, cx);
    let replacement_key = cx.update(|cx| replacement.read(cx).document().key().clone());
    assert_ne!(replacement_key.environment(), original_key.environment());
    cx.update(|cx| {
        assert!(!original.read(cx).source_current());
        assert!(replacement.read(cx).source_current());
    });
}
fn refresh_sources(f: &Fixture, cx: &mut TestAppContext) {
    cx.update(|cx| {
        f.installation
            .installed
            .runtime()
            .refresh_environment_sources(cx)
    });
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}
fn shown(f: &Fixture, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(f.handle, |_, window, _| window.try_find(id).is_some())
        .unwrap()
}
#[gpui_kit::test]
fn a_locked_hub_account_shows_an_unlock_notice_and_clears_it_after_unlock(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    let f = account_fixture(factory.clone(), cx);
    refresh_sources(&f, cx);
    assert!(shown(&f, "agent-account-notice", cx));
    assert!(
        !shown(&f, "agent-error", cx),
        "locking is not an error line"
    );

    factory.sign_in(account("alice"));
    refresh_sources(&f, cx);
    assert!(!shown(&f, "agent-account-notice", cx));
    assert!(!shown(&f, "agent-error", cx));
}
#[gpui_kit::test]
fn locking_after_capture_shows_the_notice_again(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(account("alice"));
    let f = account_fixture(factory.clone(), cx);
    refresh_sources(&f, cx);
    assert!(!shown(&f, "agent-account-notice", cx));

    factory.close();
    refresh_sources(&f, cx);
    assert!(shown(&f, "agent-account-notice", cx));
    assert!(!shown(&f, "agent-error", cx));
}
#[gpui_kit::test]
fn the_unlock_button_asks_to_open_the_unlock_dialog(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    let f = account_fixture(factory, cx);
    refresh_sources(&f, cx);
    let unlocks = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = unlocks.clone();
    cx.update_window(f.handle, |_, window, cx| {
        cx.on_action(move |_: &nocterm_workspace::UnlockVault, _| counter.set(counter.get() + 1));
        window.click("agent-account-action", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(unlocks.get(), 1);
}
#[gpui_kit::test]
fn a_signed_out_hub_account_shows_a_notice_with_a_sign_in_button(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(account("alice"));
    let f = account_fixture(factory.clone(), cx);
    factory.sign_out();
    refresh_sources(&f, cx);
    assert!(shown(&f, "agent-account-notice", cx));
    assert!(shown(&f, "agent-account-action", cx));
    assert!(!shown(&f, "agent-error", cx));
}
#[gpui_kit::test]
fn old_approval_cannot_send_input_after_synchronous_account_replacement(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(account("alice"));
    let f = account_fixture(factory.clone(), cx);
    new_chat(&f, cx);
    let thread = current(&f, cx);
    let terminal = terminal_id(&thread, cx);
    let reply = submit(
        &f,
        &thread,
        TerminalCall::SendInput(nocterm_ai::SendInput {
            terminal_id: terminal,
            text: "must not reach B".into(),
            press_enter: true,
        }),
        false,
        cx,
    );
    factory.sign_in(account("bob"));
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let token = thread.tool_cards()[0].token();
            thread.approve_tool(token, true, false, cx);
        })
    });
    cx.run_until_parked();
    assert!(result(reply).is_err());
    assert!(f.access.sent().borrow().is_empty());
    assert_eq!(f.access.link().reconnects().get(), 0);
}
#[gpui_kit::test]
fn exec_accepted_before_replacement_is_denied_before_its_first_native_poll(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(account("alice"));
    let f = account_fixture(factory.clone(), cx);
    let programs = Arc::new(Programs::new(f._directory.jobs.clone()));
    *f.access.executor().borrow_mut() = Some(programs.clone());
    new_chat(&f, cx);
    let thread = current(&f, cx);
    let terminal = terminal_id(&thread, cx);
    let _reply = submit(
        &f,
        &thread,
        TerminalCall::ExecCommand(request(&terminal)),
        false,
        cx,
    );
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let token = thread.tool_cards()[0].token();
            thread.approve_tool(token, true, false, cx);
            // No background executor turn occurs between admission and revocation.
            factory.sign_in(account("bob"));
        })
    });
    cx.run_until_parked();
    assert_eq!(
        programs.requests().len(),
        1,
        "the job was admitted before replacement"
    );
    assert_eq!(
        f._directory.jobs.snapshot(0).state(),
        nocterm_session::command_jobs::CommandState::Starting,
        "the owned runner never reached its first transport poll"
    );
    cx.background_executor
        .advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(
        f._directory.jobs.snapshot(0).state(),
        nocterm_session::command_jobs::CommandState::Cancelled
    );
    assert!(f.access.sent().borrow().is_empty());
}
#[gpui_kit::test]
fn a_message_in_a_hub_account_chat_reaches_the_agent(cx: &mut TestAppContext) {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(account("alice"));
    let f = account_fixture(factory.clone(), cx);
    refresh_sources(&f, cx);
    new_chat(&f, cx);
    let thread = current(&f, cx);
    cx.update(|cx| thread.update(cx, |thread, cx| thread.send("hello".into(), cx)));
    cx.run_until_parked();
    assert!(f.commands.pending.lock().unwrap().is_some());
    cx.update(|cx| assert!(!thread.read(cx).queue_dispatchable()));
}
