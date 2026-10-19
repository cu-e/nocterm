//! Client-side protocol handlers: session notifications and permission requests.
use std::sync::{Arc, Mutex};

use agent_client_protocol::{Agent, Client};
use agent_client_protocol_schema::v1 as acp;
use futures::future::{Either, select};
use nocterm_ai::{AgentEvent, model::auth::PermissionOutcome};

use crate::{mapping, models::Models};

#[cfg(test)]
pub(super) fn client_builder(
    events_tx: async_channel::Sender<AgentEvent>,
) -> agent_client_protocol::Builder<Client, impl agent_client_protocol::HandleDispatchFrom<Agent>> {
    client_builder_with_models(events_tx, Arc::new(Mutex::new(Models::default())))
}

pub(super) fn client_builder_with_models(
    events_tx: async_channel::Sender<AgentEvent>,
    models: Arc<Mutex<Models>>,
) -> agent_client_protocol::Builder<Client, impl agent_client_protocol::HandleDispatchFrom<Agent>> {
    let notification_tx = events_tx.clone();
    let permission_tx = events_tx;
    Client
        .builder()
        .name("nocterm")
        .on_receive_notification(
            async move |mut notification: acp::SessionNotification, _| {
                if let acp::SessionUpdate::ConfigOptionUpdate(update) = &mut notification.update {
                    update.config_options =
                        models.lock().unwrap_or_else(|e| e.into_inner()).retain(
                            &notification.session_id,
                            std::mem::take(&mut update.config_options),
                        );
                }
                // Updates the neutral model cannot hold are dropped.
                let Ok(notice) = mapping::notice(notification) else {
                    return Ok(());
                };
                notification_tx
                    .try_send(AgentEvent::Session(notice))
                    .map_err(|_| acp::Error::internal_error().data("Agent event queue overflow"))
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: acp::RequestPermissionRequest, responder, connection| {
                let Ok(request) = mapping::permission(request) else {
                    responder.respond(acp::RequestPermissionResponse::new(
                        acp::RequestPermissionOutcome::Cancelled,
                    ))?;
                    return Ok(());
                };
                let (respond, answer) = nocterm_ai::PermissionResponder::channel();
                permission_tx
                    .try_send(AgentEvent::Permission { request, respond })
                    .map_err(|_| acp::Error::internal_error())?;
                connection.clone().spawn(async move {
                    let cancellation = responder.cancellation();
                    let wait = cancellation.run_until_cancelled(async {
                        Ok(answer.await.unwrap_or(PermissionOutcome::Cancelled))
                    });
                    let outcome = match select(
                        Box::pin(wait),
                        Box::pin(connection.incoming_closed()),
                    )
                    .await
                    {
                        Either::Left((Ok(answer), _)) => answer,
                        _ => PermissionOutcome::Cancelled,
                    };
                    responder.respond(acp::RequestPermissionResponse::new(mapping::wire_outcome(
                        outcome,
                    )))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
}
