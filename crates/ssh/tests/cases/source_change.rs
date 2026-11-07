//! Original source gates persist into retained native ports and pending workers.
use super::*;
use nocterm_session::{
    ConnectionOpenOptions, ExecError, ExecRequest, FsError,
    testing::{ScriptedConnectionSource, ScriptedRelay},
};
#[tokio::test]
async fn retained_ssh_ports_and_input_refuse_a_revoked_original_source() {
    let Some(sshd) = Sshd::start() else { return };
    let source = ScriptedConnectionSource::default();
    let transport = sshd.transport("plain");
    let session = transport.open_with(sshd.request(), ConnectionOpenOptions::new(source.source()));
    connect(&session, accept).await;
    let fs = session.fs().unwrap();
    let exec = session.exec().unwrap();
    let file = sshd.root().join("must-not-exist");
    let pending = exec.exec(ExecRequest::new("touch").arg(file.to_str().unwrap()));
    let pending_fs = fs.create_dir(file.to_str().unwrap());
    source.revoke();
    assert!(matches!(pending.await, Err(ExecError::Source(_))));
    assert!(matches!(pending_fs.await, Err(FsError::Source(_))));
    assert!(!session.input(format!("touch '{}'\r", file.display())));
    assert!(!file.exists());
    assert!(matches!(
        closed(&session, no_prompts).await,
        CloseReason::Failed(SessionError::Source(_))
    ));
}
#[tokio::test]
async fn a_source_revoked_while_relay_open_is_pending_cannot_resume_the_worker() {
    let Some(sshd) = Sshd::start() else { return };
    let source = ScriptedConnectionSource::default();
    let relay = std::sync::Arc::new(ScriptedRelay::default());
    relay.reply(nocterm_session::testing::RelayReply::Hang);
    let transport = sshd.transport("plain").with_relay(relay.clone());
    let mut request = sshd.request();
    request.proxy = nocterm_session::ProxyConfig::Hub {
        address: "hub.test".into(),
    };
    let session = transport.open_with(
        request,
        ConnectionOpenOptions::new(source.source()).with_relay(relay.clone()),
    );
    timeout(Duration::from_secs(10), async {
        while relay.asked().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    source.revoke();
    assert!(!session.input("must not resume"));
    assert!(matches!(
        closed(&session, no_prompts).await,
        CloseReason::Failed(SessionError::Source(_))
    ));
    timeout(Duration::from_secs(10), async {
        while relay.abandoned_dials() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(relay.asked().len(), 1);
}

#[tokio::test]
async fn a_scoped_hub_connection_cannot_acquire_an_ambient_relay_requester() {
    let Some(sshd) = Sshd::start() else { return };
    let source = ScriptedConnectionSource::default();
    let ambient = std::sync::Arc::new(ScriptedRelay::default());
    let transport = sshd.transport("plain").with_relay(ambient.clone());
    let mut request = sshd.request();
    request.proxy = nocterm_session::ProxyConfig::Hub {
        address: "another-hub".into(),
    };
    let session = transport.open_with(request, ConnectionOpenOptions::new(source.source()));
    assert!(matches!(
        closed(&session, no_prompts).await,
        CloseReason::Failed(SessionError::Source(
            nocterm_session::ConnectionSourceError::Unsupported
        ))
    ));
    assert!(ambient.asked().is_empty());
}
