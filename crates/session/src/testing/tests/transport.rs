use crate::testing::{InMemoryFs, ScriptedTransport};
use crate::{Auth, CloseReason, Command, ConnectRequest, Event, PtySize, Target, Transport};
use futures::{FutureExt as _, executor::block_on, join};
use std::{sync::Arc, time::Duration};
fn request() -> ConnectRequest {
    ConnectRequest {
        proxy: Default::default(),
        launch: Default::default(),
        target: Target {
            host: "host".into(),
            port: 22,
            user: "me".into(),
        },
        auth: Auth::Password,
        term: "xterm".into(),
        size: Default::default(),
        connect_timeout: Duration::from_secs(10),
        keepalive_interval: None,
    }
}

#[test]
fn prepared_sessions_use_real_commands_events_and_remote_filesystem() {
    let transport = ScriptedTransport::default();
    let fs = InMemoryFs::default();
    fs.seed_file("/file", b"data").unwrap();
    let driver = transport.queue(Some(Arc::new(fs)), None);
    let session = transport.open(request());
    assert_eq!(transport.requests(), [request()]);
    assert!(session.input("ls\r"));
    session.resize(PtySize {
        cols: 120,
        ..Default::default()
    });
    block_on(async {
        assert!(matches!(
            driver.next_command().await,
            Some(Command::Resize(PtySize { cols: 120, .. }))
        ));
        assert_eq!(
            driver.next_command().await,
            Some(Command::Input(b"ls\r".to_vec()))
        );
        assert!(driver.emit(Event::Connected).await);
        assert!(driver.emit(Event::Output(b"ready".to_vec())).await);
        assert!(matches!(session.next_event().await, Some(Event::Connected)));
        assert!(
            matches!(session.next_event().await, Some(Event::Output(bytes)) if bytes == b"ready")
        );
        assert_eq!(
            session
                .fs()
                .unwrap()
                .stat("/file")
                .await
                .unwrap()
                .unwrap()
                .size,
            Some(4)
        );
    });
    session.close();
    block_on(async {
        join!(driver.closed(), driver.closed());
        assert_eq!(driver.next_command().await, Some(Command::Close));
        assert!(driver.emit(Event::Closed(CloseReason::ClosedByUser)).await);
        assert!(matches!(
            session.next_event().await,
            Some(Event::Closed(CloseReason::ClosedByUser))
        ));
    });
}

#[test]
fn driver_notices_session_drop_and_unscripted_open_fails_without_hanging() {
    let transport = ScriptedTransport::default();
    let driver = transport.queue(None, None);
    let session = transport.open(request());
    let mut closing = Box::pin(driver.closed());
    assert!(closing.as_mut().now_or_never().is_none());
    drop(session);
    block_on(closing);
    assert_eq!(block_on(driver.next_command()), None);
    assert!(matches!(
        block_on(transport.open(request()).next_event()),
        Some(Event::Closed(CloseReason::Failed(
            crate::SessionError::Cancelled
        )))
    ));
}
