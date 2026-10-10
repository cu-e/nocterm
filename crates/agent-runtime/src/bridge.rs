//! Registration ownership and one bridge event pump, separate from persistence.
use super::{Client, Runtime, SessionEvent};
use gpui::{Context, Task};
use nocterm_ai::ToolBridge;
use std::{collections::HashMap, sync::Arc};
#[derive(Default)]
pub(super) struct BridgeRegistry {
    registrations: HashMap<u64, Client>,
    task: Option<Task<()>>,
    #[cfg(any(test, feature = "test-support"))]
    test_events: Option<Task<()>>,
}
impl BridgeRegistry {
    pub(super) fn insert(&mut self, id: u64, client: Client) {
        self.registrations.insert(id, client);
    }
    pub(super) fn remove(&mut self, id: u64) {
        self.registrations.remove(&id);
    }
    fn target(&self, id: u64) -> Option<Client> {
        self.registrations
            .get(&id)
            .filter(|client| client.alive())
            .cloned()
    }
    pub(super) fn stop(&mut self) {
        self.registrations.clear();
        self.task.take();
        #[cfg(any(test, feature = "test-support"))]
        self.test_events.take();
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn install_test_events(&mut self, task: Task<()>) {
        self.test_events = Some(task);
    }
    pub(super) fn start(&mut self, bridge: Arc<dyn ToolBridge>, cx: &mut Context<Runtime>) {
        if self.task.is_some() {
            return;
        }
        let calls = bridge.calls();
        let rejections = bridge.rejections();
        self.task = Some(cx.spawn(async move |runtime, cx| {
            loop {
                use futures::future::{Either, select};
                let event = if rejections.is_closed() && rejections.is_empty() {
                    match calls.recv().await {
                        Ok(call) => SessionEvent::Tool(call),
                        Err(_) => break,
                    }
                } else {
                    match select(Box::pin(calls.recv()), Box::pin(rejections.recv())).await {
                        Either::Left((Ok(call), _)) => SessionEvent::Tool(call),
                        Either::Right((Ok(rejected), _)) => SessionEvent::ToolRejected(rejected),
                        Either::Left((Err(_), _)) => break,
                        Either::Right((Err(_), _)) => continue,
                    }
                };
                let registration = match &event {
                    SessionEvent::Tool(call) => call.registration_id,
                    SessionEvent::ToolRejected(rejected) => rejected.registration_id,
                    _ => unreachable!(),
                };
                let client = runtime
                    .read_with(cx, |runtime, _| runtime.bridge.target(registration))
                    .ok()
                    .flatten();
                if let Some(client) = client {
                    cx.update(|cx| client.emit(event, cx));
                } else if let SessionEvent::Tool(call) = event {
                    let _ = call
                        .respond
                        .send(Err("Chat was closed or AI is disabled.".into()));
                }
            }
        }));
    }
}
