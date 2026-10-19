//! Actual SDK routing and injected byte IO preserve original frame order.
use super::fixture;
use crate::managed::{
    budget::SourceBudget,
    driver::{DriverError, DriverTask, Handler},
};
use agent_client_protocol::managed::{Dispatch, Driver};
use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
struct Incoming(Arc<Mutex<Vec<String>>>);
impl Handler for Incoming {
    fn dispatch(
        &mut self,
        dispatch: Dispatch<SourceBudget>,
        _: &Driver<SourceBudget>,
    ) -> Result<(), DriverError> {
        self.0
            .lock()
            .unwrap()
            .push(dispatch.envelope().method().unwrap().into());
        dispatch.unhandled();
        Ok(())
    }
    fn poll_work(&mut self, _: &mut Context<'_>) -> Poll<Result<(), DriverError>> {
        Poll::Pending
    }
    fn closed(&mut self) {}
}
#[test]
fn injected_driver_emits_ordered_original_batch_faults_and_finishes_partial_output() {
    use futures::{executor::block_on, io::Cursor};
    let fixture = fixture();
    let raw = b"[{\"jsonrpc\":\"2.0\",\"id\":\"uuid\",\"method\":\"unknown\"},null,{\"jsonrpc\":\"2.0\",\"method\":\"notice\"}]\n";
    let methods = Arc::new(Mutex::new(Vec::new()));
    let (driver, _connection) = Driver::new(fixture.budget.clone()).unwrap();
    let mut output = Cursor::new(Vec::new());
    block_on(DriverTask::new(
        driver,
        Cursor::new(raw),
        &mut output,
        Incoming(methods.clone()),
        fixture.budget.clone(),
    ))
    .unwrap();
    assert_eq!(*methods.lock().unwrap(), ["unknown", "notice"]);
    let actual: serde_json::Value = serde_json::from_slice(output.get_ref()).unwrap();
    assert_eq!(
        actual,
        serde_json::json!([
            {"jsonrpc":"2.0","id":"uuid","error":agent_client_protocol::Error::method_not_found().data("unknown")},
            {"jsonrpc":"2.0","id":null,"error":agent_client_protocol::Error::invalid_request()},
        ])
    );
}
