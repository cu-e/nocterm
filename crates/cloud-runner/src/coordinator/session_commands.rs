//! Session admission releases command ingress before a real bounded handshake.
use super::{
    chat::Chat, run_session::SessionActivationFence, services::CoordinatorServices,
    session_publication::SessionFactPublisher,
};
use nocterm_cloud_model::{coordinator::*, journal::CommandReceipt};
use nocterm_core::allocation::Allocated;
use std::sync::Arc;

pub(super) struct SessionCommands {
    services: Arc<CoordinatorServices>,
}
impl SessionCommands {
    pub(super) fn new(services: Arc<CoordinatorServices>) -> Self {
        Self { services }
    }
    pub(super) async fn begin(
        &self,
        chat: &Arc<Chat>,
        command: &ReceivedCommand,
        request: &SessionRequest,
        work: &super::intake::PublicationWork,
    ) -> Result<CommandReceipt, CoordinatorError> {
        let setup = self.services.session_setup()?;
        let run = chat.run(request.binding().run())?;
        run.control().check(self.services.source().as_ref())?;
        run.sessions()
            .check_begin(request, self.services.timer().as_ref())?;
        let request_allocation = setup.request_metadata()?;
        let binding_allocation = setup.pending_metadata()?;
        let request = Arc::new(Allocated::new(request.clone(), request_allocation));
        let receipt = chat.accept(command, None, vec![], work).await?;
        // A competing urgent Stop can fence while commit waits. It cannot be
        // undone by this original accepted command or an eventual SSH callback.
        run.control().check(self.services.source().as_ref())?;
        run.invalidate_session_approval();
        let ticket = run.sessions().begin(
            request.clone(),
            binding_allocation,
            self.services.timer().as_ref(),
        )?;
        let fence = SessionActivationFence::new(
            self.services.source().clone(),
            run.control().clone(),
            chat.log().clone(),
            self.services.timer().clone(),
        );
        let sink = Arc::new(SessionFactPublisher::new(
            self.services.storage().clone(),
            fence,
            chat.log().clone(),
            run.sessions().clone(),
            ticket.clone(),
            setup.publication(),
            receipt.clone(),
        ));
        match setup
            .broker()
            .begin(command.acknowledgement(), request, sink.clone())
        {
            Ok(cancellation) => run.sessions().attach_cancellation(&ticket, cancellation),
            Err(error) => {
                let reason = match error {
                    CoordinatorError::Overloaded => SessionFailure::Capacity,
                    CoordinatorError::Unsupported => SessionFailure::Unsupported,
                    CoordinatorError::Expired => SessionFailure::Expired,
                    _ => SessionFailure::Unavailable,
                };
                // Admission is already durable; the immutable receipt remains
                // accepted even if the subsequent original connection refuses.
                sink.refused(reason).await?;
            }
        }
        Ok(receipt)
    }
}
