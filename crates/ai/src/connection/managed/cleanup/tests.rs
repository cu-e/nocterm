//! One shutdown attempt starts on its first poll, is shared by every waiter and
//! is retried by exactly one final attempt after a failure.
use super::*;
use crate::{
    history::source::SourceProducer,
    model::retention::{RetentionAuthority, document::DocumentRetention},
    testing::ScriptedCommands,
};
use futures::FutureExt;

struct Fixture {
    script: ScriptedCommands,
    cleanup: ManagedProcessCleanup,
    _producer: SourceProducer,
    _authority: RetentionAuthority,
}
fn fixture(script: ScriptedCommands) -> Fixture {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lifetime = producer.capture().unwrap().into_lifetime().unwrap();
    let commands = script.handle(&lifetime).unwrap();
    script.release_handle();
    let cleanup = ManagedProcessCleanup::new(commands, lifetime).unwrap();
    Fixture {
        script,
        cleanup,
        _producer: producer,
        _authority: authority,
    }
}
/// Drives `waiter`'s attempt to completion on the current thread.
fn drive(waiter: &mut ManagedShutdownWaiter) {
    let driver = waiter.take_driver().expect("first poll owns the driver");
    futures::executor::block_on(driver);
}

#[test]
fn a_dormant_cleanup_that_is_never_driven_performs_no_shutdown() {
    let f = fixture(ScriptedCommands::default());
    let waiter = f.cleanup.join().unwrap();
    drop(waiter);
    drop(f.cleanup);
    assert_eq!(f.script.shutdowns(), 0);
}

#[test]
fn the_first_poll_drives_exactly_once_for_two_waiters() {
    let f = fixture(ScriptedCommands::default());
    let mut first = f.cleanup.join().unwrap();
    let mut second = f.cleanup.join().unwrap();
    drive(&mut first);
    assert!(second.take_driver().is_none());
    assert_eq!(f.script.shutdowns(), 1);
    assert!(matches!(first.result(), Some(Ok(()))));
    assert!(matches!(futures::executor::block_on(second), Ok(())));
}

#[test]
fn a_waiter_dropped_after_the_first_poll_does_not_cancel_the_attempt() {
    let script = ScriptedCommands::default();
    let gate = script.gate_shutdown();
    let f = fixture(script);
    let mut first = f.cleanup.join().unwrap();
    let mut driver = Box::pin(first.take_driver().unwrap());
    assert!(driver.as_mut().now_or_never().is_none());
    drop(first);
    let later = f.cleanup.join().unwrap();
    gate.send(()).unwrap();
    futures::executor::block_on(driver);
    assert_eq!(f.script.shutdowns(), 1);
    assert!(matches!(futures::executor::block_on(later), Ok(())));
}

#[test]
fn a_failed_normal_cleanup_gets_exactly_one_final_attempt() {
    let script = ScriptedCommands::default();
    script.fail_shutdown();
    let f = fixture(script);
    let mut normal = f.cleanup.join().unwrap();
    drive(&mut normal);
    assert!(matches!(normal.result(), Some(Err(_))));
    let mut last = f.cleanup.capture().unwrap();
    drive(&mut last);
    assert_eq!(f.script.shutdowns(), 2);
    let mut again = f.cleanup.capture().unwrap();
    assert!(again.take_driver().is_none());
    assert!(futures::executor::block_on(again).is_err());
    assert_eq!(f.script.shutdowns(), 2);
}

#[test]
fn a_capture_during_a_pending_cleanup_joins_the_same_attempt() {
    let script = ScriptedCommands::default();
    let gate = script.gate_shutdown();
    let f = fixture(script);
    let mut running = f.cleanup.join().unwrap();
    let mut driver = Box::pin(running.take_driver().unwrap());
    assert!(driver.as_mut().now_or_never().is_none());
    let mut captured = f.cleanup.capture().unwrap();
    assert!(captured.take_driver().is_none());
    gate.send(()).unwrap();
    futures::executor::block_on(driver);
    assert!(matches!(futures::executor::block_on(captured), Ok(())));
    assert!(matches!(futures::executor::block_on(running), Ok(())));
    assert_eq!(f.script.shutdowns(), 1);
}
