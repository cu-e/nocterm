//! One shared permit bounds payloads in flight, including GUI delivery.
#[derive(Clone)]
pub struct HistoryGate {
    sender: async_channel::Sender<()>,
    receiver: async_channel::Receiver<()>,
}
impl Default for HistoryGate {
    fn default() -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        sender.try_send(()).expect("empty permit channel");
        Self { sender, receiver }
    }
}
impl HistoryGate {
    pub async fn acquire(&self) -> HistoryPermit {
        self.receiver.recv().await.expect("gate owns sender");
        HistoryPermit(self.sender.clone())
    }
}
pub struct HistoryPermit(async_channel::Sender<()>);
impl Drop for HistoryPermit {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

mod completion;
mod coordinator;
mod phase;
mod source;
pub use completion::{NativeLoadError, ReaderCompletion, ReaderFailure, ReaderOutcome};
pub use coordinator::PreparedReaderLoad;
pub use phase::{NativeMaterialized, NativePhase};
pub use source::{
    ManagedReaderFuture, ReaderDelivery, ReaderDocumentId, ReaderInstallError, ReaderMaterializer,
    ReaderPreparationFailure, ReaderRequest, ReaderSource, ReaderSourceError,
};
