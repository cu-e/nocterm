//! Real SSHD proof login, no ambient credential fallback, and live-session independence.
use super::login_fixture::LoginSetup;
use super::*;
use nocterm_cloud_model::{
    authority::TargetBinding,
    identity::Fingerprint,
    login::{LoginConsentBinding, LoginConsentWindow, LoginError, LoginProofRequest},
};
use nocterm_session::{ConnectionFailureKind, ConnectionOpenOptions, ExecRequest};
use nocterm_ssh_keys::login::UserauthLogin;

pub(super) async fn proof_requested(setup: &LoginSetup) {
    timeout(Duration::from_secs(10), async {
        while setup.requests().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn exact_device_proof_authenticates_real_ssh_and_sftp_with_literal_exec_arguments() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    let transport = sshd.transport("plain");
    let mut request = setup.request();
    let workspace = sshd.root().join("workspace with ' quote");
    fs::create_dir(&workspace).unwrap();
    request.launch.cwd = Some(workspace.to_string_lossy().into_owned());
    request.launch.integration = false;
    let session = transport.open_login(request, Default::default(), setup.attempt());
    connect(&session, no_prompts).await;
    assert_eq!(setup.reads(), 1);
    assert_eq!(setup.requests().len(), 1);
    let fs = session.fs().unwrap();
    assert!(
        fs.stat(workspace.to_str().unwrap())
            .await
            .unwrap()
            .is_some()
    );
    session.input("printf 'PTY_PWD='; pwd\r");
    let text = output_containing(&session, workspace.to_str().unwrap()).await;
    assert!(text.contains("PTY_PWD="));
    let exec = session.exec().unwrap();
    let pwd = exec
        .exec(ExecRequest::new("pwd"))
        .await
        .unwrap()
        .collect(4096)
        .await;
    assert!(pwd.exit.success());
    assert_ne!(
        pwd.text().trim(),
        workspace.to_str().unwrap(),
        "separate exec is not a workspace sandbox"
    );
    let literal = "$(touch should-not-exist); ' value\nsecond";
    let output = exec
        .exec(ExecRequest::new("printf").arg("%s").arg(literal))
        .await
        .unwrap()
        .collect(4096)
        .await;
    assert!(output.exit.success());
    assert_eq!(output.text(), literal);
    session.close();
    closed(&session, no_prompts).await;
}
#[tokio::test]
async fn real_server_rekeys_after_device_revocation_and_authorizer_drop() {
    let Some(sshd) = Sshd::start_with("RekeyLimit 1K 1s\n") else {
        return;
    };
    let setup = LoginSetup::new(&sshd);
    let observer = setup.observer();
    let transport = sshd.transport("empty");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    connect(&session, no_prompts).await;
    assert_eq!(setup.reads(), 1);
    let before = sshd.completed_key_exchanges();
    assert!(before >= 1);
    setup.revoke_device();
    drop(setup);
    assert!(
        observer.upgrade().is_none(),
        "authenticated R session retained the device authorizer"
    );
    let file = sshd.root().join("rekey-payload");
    fs::write(&file, vec![b'x'; 512 * 1024]).unwrap();
    let fs = session.fs().unwrap();
    let mut download = fs.download(file.to_str().unwrap()).await.unwrap();
    let mut received = 0;
    loop {
        let part = download.read(32 * 1024).await.unwrap();
        if part.is_empty() {
            break;
        }
        received += part.len();
    }
    assert_eq!(received, 512 * 1024);
    download.close().await.unwrap();
    timeout(Duration::from_secs(10), async {
        while sshd.completed_key_exchanges() <= before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real server did not complete another key exchange");
    let output = session
        .exec()
        .unwrap()
        .exec(ExecRequest::new("printf").arg("still authenticated"))
        .await
        .unwrap()
        .collect(100)
        .await;
    assert_eq!(output.text(), "still authenticated");
    assert!(output.exit.success());
    session.close();
    closed(&session, no_prompts).await;
}
#[tokio::test]
async fn expired_or_replayed_consent_never_falls_back_to_ambient_authorized_keys() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    let transport = sshd.transport("plain");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    connect(&session, no_prompts).await;
    session.close();
    closed(&session, no_prompts).await;
    let replay = transport.open_login(setup.request(), Default::default(), setup.attempt());
    let reason = closed(&replay, no_prompts).await;
    assert!(
        matches!(reason, CloseReason::Failed(SessionError::Connection(_))),
        "{reason:?}"
    );
    assert_eq!(setup.reads(), 1);
    let fresh = LoginSetup::new(&sshd);
    fresh.expire();
    let expired = transport.open_login(fresh.request(), Default::default(), fresh.attempt());
    assert!(
        matches!(closed(&expired,no_prompts).await,CloseReason::Failed(SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::NeedsUnlock)
    );
    assert_eq!(fresh.reads(), 0);
}
#[tokio::test]
async fn consent_expiring_during_original_credential_lookup_cannot_sign() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    let gate = nocterm_session::testing::Gate::pending();
    setup.suspend(gate.clone());
    let transport = sshd.transport("plain");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    proof_requested(&setup).await;
    setup.expire();
    gate.release();
    assert!(
        matches!(closed(&session,no_prompts).await,CloseReason::Failed(SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::NeedsUnlock)
    );
}
#[tokio::test]
async fn actual_host_proof_rejects_substitution_before_any_device_key_lookup() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    setup.pause_before_device();
    let transport = sshd.transport("plain");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    proof_requested(&setup).await;
    session.close();
    closed(&session, no_prompts).await;
    let original = setup.requests().pop().unwrap();
    let b = original.binding();
    let mut payload = original.login().payload().to_vec();
    payload[4] ^= 1;
    let wrong_session = LoginProofRequest::new(
        b.clone(),
        original.host().clone(),
        UserauthLogin::parse(payload).unwrap(),
    );
    assert!(setup.authorize(wrong_session).await.is_err());
    let target = TargetBinding::new(
        b.target().host(),
        Fingerprint::from_bytes([99; 32]),
        b.target().principal().clone(),
        b.target().workspace(),
    );
    let wrong_route = LoginConsentBinding::new(
        b.run().clone(),
        target,
        b.authentication().clone(),
        b.attempt(),
        *b.nonce(),
        LoginConsentWindow::new(b.issued(), b.expires()).unwrap(),
        b.user_key().clone(),
    )
    .unwrap();
    assert!(matches!(
        setup
            .authorize(LoginProofRequest::new(
                wrong_route,
                original.host().clone(),
                original.login().clone()
            ))
            .await,
        Err(LoginError::Mismatch)
    ));
    let wrong_nonce = LoginConsentBinding::new(
        b.run().clone(),
        b.target().clone(),
        b.authentication().clone(),
        b.attempt(),
        [88; 32],
        LoginConsentWindow::new(b.issued(), b.expires()).unwrap(),
        b.user_key().clone(),
    )
    .unwrap();
    assert!(matches!(
        setup
            .authorize(LoginProofRequest::new(
                wrong_nonce,
                original.host().clone(),
                original.login().clone()
            ))
            .await,
        Err(LoginError::Mismatch)
    ));
    assert_eq!(setup.reads(), 0);
    setup.authorize(original.clone()).await.unwrap();
    assert_eq!(setup.reads(), 1);
    assert!(matches!(
        setup.authorize(original).await,
        Err(LoginError::Replay)
    ));
    assert_eq!(setup.reads(), 1);
}
#[tokio::test]
async fn unsupported_proxy_route_fails_before_the_authorizer_is_contacted() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    let transport = sshd.transport("plain");
    let mut request = setup.request();
    request.proxy = nocterm_session::ProxyConfig::HttpConnect {
        host: "127.0.0.1".into(),
        port: 1,
    };
    let session = transport.open_login(request, ConnectionOpenOptions::default(), setup.attempt());
    assert!(
        matches!(closed(&session,no_prompts).await,CloseReason::Failed(SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::Unsupported)
    );
    assert!(setup.requests().is_empty());
    assert_eq!(setup.reads(), 0);
}

#[tokio::test]
async fn cancelling_a_pending_device_signature_closes_the_real_native_handshake() {
    let Some(sshd) = Sshd::start() else { return };
    let setup = LoginSetup::new(&sshd);
    setup.pause_before_device();
    let observer = setup.observer();
    let before = sshd.unauthenticated_disconnects();
    let transport = sshd.transport("plain");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    proof_requested(&setup).await;
    drop(setup);
    session.close();
    closed(&session, no_prompts).await;
    timeout(Duration::from_secs(10), async {
        while observer.upgrade().is_some() || sshd.unauthenticated_disconnects() <= before {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancelled proof retained a native worker or device authorizer");
}
#[tokio::test]
async fn unsupported_initial_kex_never_contacts_the_device_or_uses_ambient_keys() {
    let Some(sshd) = Sshd::start_with("KexAlgorithms diffie-hellman-group14-sha256\n") else {
        return;
    };
    let setup = LoginSetup::new(&sshd);
    let transport = sshd.transport("plain");
    let session = transport.open_login(setup.request(), Default::default(), setup.attempt());
    assert!(
        matches!(closed(&session,no_prompts).await,CloseReason::Failed(SessionError::Connection(error)) if error.kind()==ConnectionFailureKind::Unsupported)
    );
    assert!(setup.requests().is_empty());
    assert_eq!(setup.reads(), 0);
}
