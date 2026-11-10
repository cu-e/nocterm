//! Program actions inherit the original host source and never capture another window.
use super::*;
use nocterm_hubs_model::{
    source_identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition,
        LocalAccountSourceFactory as _, NamespaceId,
    },
    testing::ScriptedLocalAccountFactory,
};
use nocterm_session::{ExecRequest, HostExec, SessionDriver, Target, testing::FakeExec};
use nocterm_workspace::{
    Host, ItemHandle as _, ProgramSpec, SessionContext, SessionSource, TerminalInfo,
};
fn partition(hub: &str) -> LocalAccountPartition {
    LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new(hub).unwrap(),
            Fingerprint::from_bytes([1; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("same-account").unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    ))
}
fn account(hub: &str) -> Arc<ScriptedLocalAccountFactory> {
    let factory = Arc::new(ScriptedLocalAccountFactory::default());
    factory.sign_in(partition(hub));
    factory
}
fn other_hub_provider(
    captures: Arc<AtomicUsize>,
    other: Arc<ScriptedLocalAccountFactory>,
) -> crate::CredentialProvider {
    crate::CredentialProvider::new(Arc::new(FakeCredentialStore::default()), |_, _, _| {
        Task::ready(Ok(()))
    })
    .with_capture(move || {
        captures.fetch_add(1, Ordering::SeqCst);
        Ok(crate::CredentialCapture::account(
            Arc::new(FakeCredentialStore::default()),
            other.capture().unwrap(),
        ))
    })
}
fn host(exec: Arc<FakeExec>, source: SessionSource) -> Host {
    Host::resolve(
        Some(
            &SessionContext::new(Target::new("user", "host", 22), None, true)
                .with_exec(Some(exec))
                .with_source(source),
        ),
        None,
    )
    .unwrap()
}
struct ProgramChild {
    view: Entity<TerminalView>,
    driver: Arc<SessionDriver>,
    exec: Arc<FakeExec>,
    source: SessionSource,
    factory: Arc<ScriptedLocalAccountFactory>,
    other: Arc<ScriptedLocalAccountFactory>,
    captures: Arc<AtomicUsize>,
}
impl ProgramChild {
    fn new(cx: &mut TestAppContext) -> Self {
        let factory = account("A");
        let original = crate::CredentialCapture::account(
            Arc::new(FakeCredentialStore::default()),
            factory.capture().unwrap(),
        );
        let source = original.session_source();
        assert!(source.same_as(&original.session_source()));
        let captures = Arc::new(AtomicUsize::new(0));
        let other = account("B");
        let f = super::super::fixture(cx);
        let installed = f
            .installed
            .with_credentials(other_hub_provider(captures.clone(), other.clone()));
        let exec = Arc::new(FakeExec::default());
        let driver = Arc::new(exec.queue_terminal(source.connection().clone()));
        let host = host(exec.clone(), source.clone());
        let view = cx
            .update_window(f.window, |_, window, cx| {
                let terminal = cx.new(|cx| {
                    crate::Terminal::new_program(
                        installed,
                        ProgramSpec {
                            title: "Original A".into(),
                            host,
                            program: ExecRequest::new("top"),
                            shell_syntax: None,
                        },
                        cx,
                    )
                });
                cx.new(|cx| TerminalView::with_terminal(terminal, window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        let this = Self {
            view,
            driver,
            exec,
            source,
            factory,
            other,
            captures,
        };
        this.connect(cx);
        this
    }
    fn connect(&self, cx: &mut TestAppContext) {
        let driver = self.driver.clone();
        cx.background_executor
            .spawn(async move {
                driver.emit(Event::Connected).await;
            })
            .detach();
        cx.run_until_parked();
    }
    fn info(&self, cx: &mut TestAppContext) -> TerminalInfo {
        cx.update(|cx| {
            self.view
                .terminal_capabilities(cx)
                .unwrap()
                .state()
                .info(cx)
                .unwrap()
        })
    }
    fn source(&self, cx: &mut TestAppContext) -> SessionSource {
        cx.update(|cx| {
            self.view
                .read(cx)
                .terminal()
                .read(cx)
                .session_context()
                .source()
                .clone()
        })
    }
    fn send(&self, text: &str, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| self.view.read(cx).terminal().read(cx).send(text))
    }
    fn reconnect(&self, cx: &mut TestAppContext) {
        cx.update(|cx| {
            let child = self.view.read(cx).terminal().clone();
            child.update(cx, |child, cx| child.reconnect(cx));
        });
        cx.run_until_parked();
    }
}
#[gpui_kit::test]
fn a_program_opened_from_another_valid_hub_keeps_its_original_source(cx: &mut TestAppContext) {
    let child = ProgramChild::new(cx);
    let info = child.info(cx);
    assert!(child.other.capture().unwrap().check_current().is_ok());
    assert_eq!(info.source, Some(partition("A")));
    assert_ne!(info.source, Some(partition("B")));
    assert!(info.source_current);
    assert!(child.source(cx).same_as(&child.source));
    assert!(child.source(cx).same_as(&child.source(cx)));
    assert!(child.send("A input", cx));
    assert_eq!(child.exec.terminal_requests().len(), 1);
    assert_eq!(child.captures.load(Ordering::SeqCst), 0);
}
#[gpui_kit::test]
fn program_reconnect_is_inert_and_a_b_a_never_restores_an_old_child(cx: &mut TestAppContext) {
    let child = ProgramChild::new(cx);
    child.reconnect(cx);
    assert_eq!(child.exec.terminal_requests().len(), 1);
    assert!(child.source(cx).same_as(&child.source));
    assert_eq!(child.captures.load(Ordering::SeqCst), 0);
    child.factory.close();
    child.factory.sign_in(partition("A"));
    assert!(!child.send("foreign input", cx));
    assert!(!child.info(cx).source_current);
    assert!(child.source(cx).check_current().is_err());
    child.reconnect(cx);
    assert_eq!(child.exec.terminal_requests().len(), 1);
    assert_eq!(child.captures.load(Ordering::SeqCst), 0);
}
#[gpui_kit::test]
fn program_without_a_host_session_stays_this_computer_without_hub_credentials(
    cx: &mut TestAppContext,
) {
    let f = super::super::fixture(cx);
    let calls = Arc::new(AtomicUsize::new(0));
    let installed = f
        .installed
        .with_credentials(other_hub_provider(calls.clone(), account("B")));
    let exec = Arc::new(FakeExec::default());
    let _driver = exec.queue_terminal(Default::default());
    let native: Arc<dyn HostExec> = exec.clone();
    let host = Host::resolve(None, Some(&native)).unwrap();
    let child = cx.update(|cx| {
        cx.new(|cx| {
            crate::Terminal::new_program(
                installed,
                ProgramSpec {
                    title: "This computer".into(),
                    host,
                    program: ExecRequest::new("top"),
                    shell_syntax: None,
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(exec.terminal_requests().len(), 1);
    cx.update(|cx| {
        assert!(
            child
                .read(cx)
                .session_context()
                .source()
                .partition()
                .is_none()
        )
    });
}
