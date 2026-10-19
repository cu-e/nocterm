//! Native aliases retain one paid concrete command allocation through last drop.
use super::*;
use crate::{
    history::source::{SourceLifetime, SourceProducer},
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Commands {
    drops: Arc<AtomicUsize>,
    lifetime: SourceLifetime,
    padding: [u8; 4096],
}
impl Drop for Commands {
    fn drop(&mut self) {
        let context = self.lifetime.context().unwrap();
        assert!(context.ledger().live_requested_bytes() >= std::mem::size_of::<Self>());
        std::hint::black_box(&self.padding);
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
macro_rules! unused {
    ($name:ident,$input:ty,$output:ty) => {
        fn $name(&self, _: &$input) -> Operation<$output> {
            unreachable!()
        }
    };
}
impl ManagedAgentCommands for Commands {
    unused!(new_session, SessionOpen, ManagedOpened);
    unused!(restore_session, RestoreSession, ManagedOpened);
    unused!(set_mode, ModeSelection, ());
    unused!(
        set_config_option,
        ConfigurationSelection,
        NativeOwned<Vec<ConfigOption>>
    );
    unused!(authenticate, AuthMethodId, ());
    unused!(close_session, SessionId, CloseSessionOutcome);
    fn prepare_prompt(
        &self,
        _: &DispatchPayload,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        unreachable!()
    }
    fn cancel(&self, _: &SessionId) -> Result<(), ManagedCommandError> {
        unreachable!()
    }
    fn shutdown_gracefully(&self) -> Operation<()> {
        unreachable!()
    }
    fn shutdown_finally(&self) -> Operation<()> {
        unreachable!()
    }
    fn shutdown(&self) {}
}
#[test]
fn command_aliases_preserve_original_identity_and_free_concrete_before_credit() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lifetime = producer.capture().unwrap().into_lifetime().unwrap();
    let context = lifetime.context().unwrap();
    let baseline = context.ledger().live_requested_bytes();
    let drops = Arc::new(AtomicUsize::new(0));
    let commands = ManagedCommandsHandle::new_with_lifetime(
        &context,
        lifetime.clone(),
        Commands {
            drops: drops.clone(),
            lifetime,
            padding: [7; 4096],
        },
    )
    .unwrap();
    let paid = context.ledger().live_requested_bytes();
    let alias = commands.clone();
    assert!(commands.same_connection(&alias));
    assert_eq!(context.ledger().live_requested_bytes(), paid);
    drop(commands);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(producer.capture().is_err());
    drop(alias);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(context.ledger().live_requested_bytes(), baseline);
    assert!(producer.capture().is_ok());
}
