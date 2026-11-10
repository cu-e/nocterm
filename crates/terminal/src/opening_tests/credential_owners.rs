//! Terminal creation captures one credential owner; reconnect never recaptures.
use super::*;
use gpui_kit::Task;
use nocterm_session::{Event, Prompt, Reply, Secret, SecretRequest, testing::FakeCredentialStore};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn remember(view: &Entity<TerminalView>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let terminal = view.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| {
            let (reply, _) = Reply::channel();
            terminal.handle_event(
                Event::Prompt(Prompt::Secret {
                    request: SecretRequest::Password {
                        target: terminal.spec().target.clone(),
                        retry: false,
                    },
                    reply,
                    hop: None,
                }),
                cx,
            );
            terminal.answer_secret_and_remember(Secret::new("test secret"), true, cx);
            terminal.handle_event(Event::Connected, cx);
        });
    });
    cx.run_until_parked();
}
#[gpui_kit::test]
fn reconnect_keeps_the_original_captured_store_and_new_terminals_capture_the_new_owner(
    cx: &mut TestAppContext,
) {
    let a = Arc::new(FakeCredentialStore::default());
    let b = Arc::new(FakeCredentialStore::default());
    let switched = Arc::new(AtomicBool::new(false));
    let captures = Arc::new(AtomicUsize::new(0));
    let provider = crate::CredentialProvider::new(a.clone(), |_, _, _| Task::ready(Ok(())))
        .with_capture({
            let (a, b, switched, captures) =
                (a.clone(), b.clone(), switched.clone(), captures.clone());
            move || {
                captures.fetch_add(1, Ordering::SeqCst);
                Ok(crate::CredentialCapture::this_computer(
                    if switched.load(Ordering::SeqCst) {
                        b.clone()
                    } else {
                        a.clone()
                    },
                ))
            }
        });
    let (_, original, _) = crate::view::tests::fixture_with_credentials(cx, Some(provider.clone()));
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    switched.store(true, Ordering::SeqCst);
    a.set_available(false);
    cx.update(|cx| {
        let terminal = original.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| terminal.reconnect(cx));
    });
    cx.run_until_parked();
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    remember(&original, cx);
    assert!(a.labels().is_empty());
    assert!(
        b.labels().is_empty(),
        "an old terminal cannot save to the replacement owner"
    );
    let (_, fresh, _) = crate::view::tests::fixture_with_credentials(cx, Some(provider));
    assert_eq!(captures.load(Ordering::SeqCst), 2);
    remember(&fresh, cx);
    assert_eq!(b.labels().len(), 1);
}
#[gpui_kit::test]
fn unavailable_initial_capture_remains_closed_across_reconnect(cx: &mut TestAppContext) {
    let available = Arc::new(AtomicBool::new(false));
    let captures = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(FakeCredentialStore::default());
    let provider = crate::CredentialProvider::new(store.clone(), |_, _, _| Task::ready(Ok(())))
        .with_capture({
            let (available, captures, store) = (available.clone(), captures.clone(), store.clone());
            move || {
                captures.fetch_add(1, Ordering::SeqCst);
                if available.load(Ordering::SeqCst) {
                    Ok(crate::CredentialCapture::this_computer(store.clone()))
                } else {
                    Err(nocterm_session::CredentialStoreError::Locked)
                }
            }
        });
    let (_, original, _) = crate::view::tests::fixture_with_credentials(cx, Some(provider));
    available.store(true, Ordering::SeqCst);
    cx.update(|cx| {
        let terminal = original.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| terminal.reconnect(cx));
    });
    cx.run_until_parked();
    remember(&original, cx);
    assert_eq!(captures.load(Ordering::SeqCst), 1);
    assert!(store.labels().is_empty());
}

#[gpui_kit::test]
fn captured_account_revocation_fences_terminal_capabilities_and_reconnect_without_renewing_identity(
    cx: &mut TestAppContext,
) {
    use nocterm_hubs_model::{
        source_identity::{
            AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition,
            LocalAccountSourceFactory, NamespaceId,
        },
        testing::ScriptedLocalAccountFactory,
    };
    let partition = LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("hub").unwrap(),
            Fingerprint::from_bytes([1; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("alice").unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    ));
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(partition.clone());
    let store = Arc::new(FakeCredentialStore::default());
    let provider = crate::CredentialProvider::new(store.clone(), |_, _, _| Task::ready(Ok(())))
        .with_capture({
            let factory = factory.clone();
            move || {
                Ok(crate::CredentialCapture::account(
                    store.clone(),
                    factory
                        .capture()
                        .map_err(|_| nocterm_session::CredentialStoreError::Locked)?,
                ))
            }
        });
    let (_, view, transport) = crate::view::tests::fixture_with_credentials(cx, Some(provider));
    let (capabilities, expected) = cx.update(|cx| {
        let capabilities = nocterm_workspace::ItemHandle::terminal_capabilities(&view, cx).unwrap();
        let info = capabilities.state().info(cx).unwrap();
        assert_eq!(info.source, Some(partition.clone()));
        assert!(info.source_current);
        (capabilities, info.expected)
    });
    factory.close();
    factory.sign_in(partition.clone());
    cx.update(|cx| {
        let info = capabilities.state().info(cx).unwrap();
        assert_eq!(info.source, Some(partition));
        assert!(!info.source_current);
        assert!(capabilities.fact_session().unwrap().current(cx).is_none());
        assert!(matches!(
            capabilities
                .input()
                .unwrap()
                .send_text(expected, "foreign", cx),
            Err(nocterm_workspace::TerminalError::StaleSession)
        ));
        assert!(matches!(
            capabilities.execution().unwrap().executor(expected, cx),
            Err(nocterm_workspace::TerminalError::StaleSession)
        ));
        assert!(matches!(
            capabilities.reconnect().unwrap().reconnect(expected, cx),
            Err(nocterm_workspace::TerminalError::StaleSession)
        ));
        let terminal = view.read(cx).terminal().clone();
        terminal.update(cx, |terminal, cx| terminal.reconnect(cx));
    });
    cx.run_until_parked();
    assert_eq!(
        transport.request_count(),
        1,
        "direct reconnect also refuses the stale original owner"
    );
}

mod programs;
#[gpui_kit::test]
fn an_accounts_window_keeps_the_answer_in_the_accounts_vault_without_being_asked(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    use nocterm_hubs_model::{
        source_identity::{
            AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition,
            LocalAccountSourceFactory as _, NamespaceId,
        },
        testing::ScriptedLocalAccountFactory,
    };
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("hub").unwrap(),
            Fingerprint::from_bytes([1; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("ann").unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    )));
    let store = Arc::new(FakeCredentialStore::default());
    let provider = crate::CredentialProvider::new(store.clone(), |_, _, _| Task::ready(Ok(())))
        .with_capture({
            let store = store.clone();
            move || {
                Ok(crate::CredentialCapture::account(
                    store.clone(),
                    factory
                        .capture()
                        .map_err(|_| nocterm_session::CredentialStoreError::Locked)?,
                ))
            }
        });
    let (handle, view, _) = crate::view::tests::fixture_with_credentials(cx, Some(provider));
    let terminal = cx.read(|cx| view.read(cx).terminal().clone());
    terminal.update(cx, |terminal, cx| {
        let (reply, _) = Reply::channel();
        let request = SecretRequest::Password {
            target: terminal.spec().target.clone(),
            retry: false,
        };
        terminal.handle_event(
            Event::Prompt(Prompt::Secret {
                request,
                reply,
                hop: None,
            }),
            cx,
        );
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("remember-credential").checked(), Some(true));
        window.click("secret-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    terminal.update(cx, |terminal, cx| {
        terminal.handle_event(Event::Connected, cx)
    });
    cx.run_until_parked();
    assert_eq!(store.labels().len(), 1, "kept without the box being ticked");
}
