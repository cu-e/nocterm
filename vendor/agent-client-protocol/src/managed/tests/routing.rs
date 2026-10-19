//! Raw RPC behavior through public operations, original sources and real futures.
use super::{Meter, meter};
use crate::managed::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

fn cx() -> Context<'static> {
    Context::from_waker(futures::task::noop_waker_ref())
}
fn call(budget: &Meter, id: i64) -> Envelope<Meter> {
    let raw = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"work","params":{{}}}}"#);
    let source = Source::copy(&raw, budget.clone()).unwrap();
    let start = raw.find("work").unwrap();
    let payload = source.span(raw.len() - 3..raw.len() - 1).unwrap();
    Envelope::validated(
        MessageKind::Request,
        Some(MessageId::Number(id)),
        Some(source.span(start..start + 4).unwrap()),
        Payload::Supplied(payload),
        source.full(),
    )
    .unwrap()
}
fn result(budget: &Meter, id: i64) -> Envelope<Meter> {
    let raw = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":42}}"#);
    let source = Source::copy(&raw, budget.clone()).unwrap();
    Envelope::validated(
        MessageKind::Result,
        Some(MessageId::Number(id)),
        None,
        Payload::Supplied(source.span(raw.len() - 3..raw.len() - 1).unwrap()),
        source.full(),
    )
    .unwrap()
}
fn notice(budget: &Meter) -> Envelope<Meter> {
    let raw = r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#;
    let source = Source::copy(raw, budget.clone()).unwrap();
    let start = raw.find("session/update").unwrap();
    Envelope::validated(
        MessageKind::Notification,
        None,
        Some(source.span(start..start + 14).unwrap()),
        Payload::Supplied(source.span(raw.len() - 3..raw.len() - 1).unwrap()),
        source.full(),
    )
    .unwrap()
}
fn request(connection: &Connection<Meter>, envelope: Envelope<Meter>) -> Request<Meter> {
    connection
        .request(envelope)
        .map_err(|(error, _)| error)
        .unwrap()
}
fn next_dispatch(
    reception: &mut Reception<Meter>,
    driver: &mut Driver<Meter>,
) -> Option<Dispatch<Meter>> {
    match reception.poll_dispatch(driver, &mut cx()) {
        Poll::Ready(Ok(value)) => value,
        _ => panic!("expected ready dispatch"),
    }
}
fn publish(driver: &mut Driver<Meter>) {
    match driver.poll_outgoing(&mut cx()) {
        Poll::Ready(Some(Outgoing::Message(_))) => {}
        _ => panic!("expected original outgoing source"),
    }
}
fn response(request: &mut Request<Meter>) -> Response<Meter> {
    match Pin::new(request).poll(&mut cx()) {
        Poll::Ready(Ok(response)) => response,
        _ => panic!("expected original response"),
    }
}
#[test]
fn never_polled_request_cancellation_discards_original_before_publication() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let pending = request(&connection, call(&budget, 1));
    drop(pending);
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn published_request_cancellation_keeps_its_original_id() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let pending = request(&connection, call(&budget, 37));
    publish(&mut driver);
    drop(pending);
    match driver.poll_outgoing(&mut cx()) {
        Poll::Ready(Some(Outgoing::Cancel(MessageId::Number(37)))) => {}
        _ => panic!("expected original cancellation id"),
    }
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn raw_response_outlives_driver_and_original_transport_source() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut pending = request(&connection, call(&budget, 1));
    publish(&mut driver);
    let mut reception = driver.receive(Frame::Single(result(&budget, 1))).unwrap();
    assert!(next_dispatch(&mut reception, &mut driver).is_none());
    let response = response(&mut pending);
    drop(pending);
    drop(reception);
    drop(driver);
    drop(connection);
    assert_eq!(
        response.envelope().payload().supplied().unwrap().text(),
        "42"
    );
    assert!(budget.0.lock().unwrap().live > 0);
    drop(response);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn ordered_response_guard_blocks_following_batch_dispatch_after_request_future_drop() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut pending = request(&connection, call(&budget, 1)).ordered();
    publish(&mut driver);
    let mut entries = Buffer::new(budget.clone());
    entries
        .push(Entry::Message(result(&budget, 1)))
        .map_err(|(e, _)| e)
        .unwrap();
    entries
        .push(Entry::Message(notice(&budget)))
        .map_err(|(e, _)| e)
        .unwrap();
    let mut reception = driver.receive(Frame::batch(entries).unwrap()).unwrap();
    assert!(reception.poll_dispatch(&mut driver, &mut cx()).is_pending());
    let response = response(&mut pending);
    drop(pending);
    assert!(reception.poll_dispatch(&mut driver, &mut cx()).is_pending());
    drop(response);
    let dispatch = next_dispatch(&mut reception, &mut driver).unwrap();
    assert_eq!(dispatch.envelope().method(), Some("session/update"));
    dispatch.unhandled();
    assert!(next_dispatch(&mut reception, &mut driver).is_none());
    drop(reception);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn close_fails_original_waiter_and_escaped_connection_pins_only_paid_header() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut pending = request(&connection, call(&budget, 1));
    driver.close();
    assert!(matches!(
        Pin::new(&mut pending).poll(&mut cx()),
        Poll::Ready(Err(ManagedError::Closed))
    ));
    drop(pending);
    drop(driver);
    assert!(budget.0.lock().unwrap().live > 0);
    assert!(matches!(connection.next_id(), Err(ManagedError::Closed)));
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn reused_incoming_id_keeps_stale_cancellation_and_release_detached() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut first = driver.receive(Frame::Single(call(&budget, 7))).unwrap();
    let mut first_dispatch = next_dispatch(&mut first, &mut driver).unwrap();
    let first_responder = first_dispatch.take_responder().unwrap();
    let mut second = driver.receive(Frame::Single(call(&budget, 7))).unwrap();
    let mut second_dispatch = next_dispatch(&mut second, &mut driver).unwrap();
    let second_responder = second_dispatch.take_responder().unwrap();
    assert!(driver.cancel_incoming(&MessageId::Number(7)));
    assert!(!first_responder.cancellation().unwrap().is_cancelled());
    assert!(second_responder.cancellation().unwrap().is_cancelled());
    drop(first_responder);
    assert!(second_responder.cancellation().unwrap().is_cancelled());
    drop(second_responder);
    drop(first_dispatch);
    drop(second_dispatch);
    drop(first);
    drop(second);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn batch_reply_emission_preserves_original_slot_order() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut entries = Buffer::new(budget.clone());
    entries
        .push(Entry::Message(call(&budget, 1)))
        .map_err(|(e, _)| e)
        .unwrap();
    entries
        .push(Entry::Message(call(&budget, 2)))
        .map_err(|(e, _)| e)
        .unwrap();
    let mut reception = driver.receive(Frame::batch(entries).unwrap()).unwrap();
    let mut first = next_dispatch(&mut reception, &mut driver).unwrap();
    let mut second = next_dispatch(&mut reception, &mut driver).unwrap();
    second
        .take_responder()
        .unwrap()
        .respond(result(&budget, 2))
        .map_err(|(e, _, _)| e)
        .unwrap();
    first
        .take_responder()
        .unwrap()
        .respond(result(&budget, 1))
        .map_err(|(e, _, _)| e)
        .unwrap();
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    assert!(next_dispatch(&mut reception, &mut driver).is_none());
    match driver.poll_outgoing(&mut cx()) {
        Poll::Ready(Some(Outgoing::Responses {
            batch: true,
            replies,
        })) => {
            let ids: Vec<_> = replies
                .as_slice()
                .iter()
                .map(|reply| match reply {
                    Reply::Message(envelope) => match envelope.id() {
                        Some(MessageId::Number(id)) => *id,
                        _ => panic!("expected id"),
                    },
                    _ => panic!("expected encoded reply"),
                })
                .collect();
            assert_eq!(ids, vec![1, 2]);
        }
        _ => panic!("expected original response batch"),
    }
    drop(first);
    drop(second);
    drop(reception);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}
#[test]
fn abandoned_batch_responder_waits_for_actual_handler_attempt_drop() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut entries = Buffer::new(budget.clone());
    entries
        .push(Entry::Message(call(&budget, 1)))
        .map_err(|(e, _)| e)
        .unwrap();
    let mut reception = driver.receive(Frame::batch(entries).unwrap()).unwrap();
    let mut dispatch = next_dispatch(&mut reception, &mut driver).unwrap();
    drop(dispatch.take_responder());
    assert!(next_dispatch(&mut reception, &mut driver).is_none());
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    drop(dispatch);
    match driver.poll_outgoing(&mut cx()) {
        Poll::Ready(Some(Outgoing::Responses {
            batch: true,
            replies,
        })) => {
            assert!(matches!(
                replies.as_slice(),
                [Reply::Fault {
                    fault: Fault::Abandoned,
                    ..
                }]
            ));
        }
        _ => panic!("expected original abandoned batch fault"),
    }
    drop(reception);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn original_permission_cancellation_wakes_all_prepaid_waiters() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let mut reception = driver.receive(Frame::Single(call(&budget, 3))).unwrap();
    let mut dispatch = next_dispatch(&mut reception, &mut driver).unwrap();
    let responder = dispatch.take_responder().unwrap();
    let cancellation = responder.cancellation().unwrap();
    let mut first = cancellation.cancelled().unwrap();
    let mut second = cancellation.cancelled().unwrap();
    assert!(Pin::new(&mut first).poll(&mut cx()).is_pending());
    assert!(Pin::new(&mut second).poll(&mut cx()).is_pending());
    assert!(driver.cancel_incoming(&MessageId::Number(3)));
    assert!(Pin::new(&mut first).poll(&mut cx()).is_ready());
    assert!(Pin::new(&mut second).poll(&mut cx()).is_ready());
    let allocations = budget.0.lock().unwrap().reservations;
    let mut already = cancellation.cancelled().unwrap();
    assert!(Pin::new(&mut already).poll(&mut cx()).is_ready());
    assert_eq!(budget.0.lock().unwrap().reservations, allocations);
    drop(already);
    drop(first);
    drop(second);
    drop(cancellation);
    drop(responder);
    drop(dispatch);
    drop(reception);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn refusal_before_request_publication_returns_same_original_source() {
    let budget = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let envelope = call(&budget, 8);
    let original = envelope.original().clone();
    let baseline = budget.0.lock().unwrap().live;
    budget.0.lock().unwrap().limit = baseline;
    let rejected = match connection.request(envelope) {
        Err((ManagedError::Refused(_), envelope)) => envelope,
        _ => panic!("expected original envelope refusal"),
    };
    assert!(rejected.original().source().same_source(original.source()));
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    assert_eq!(budget.0.lock().unwrap().live, baseline);
    budget.0.lock().unwrap().limit = usize::MAX;
    let pending = request(&connection, rejected);
    publish(&mut driver);
    drop(pending);
    assert!(matches!(
        driver.poll_outgoing(&mut cx()),
        Poll::Ready(Some(Outgoing::Cancel(MessageId::Number(8))))
    ));
    drop(original);
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
}

#[test]
fn original_sdk_uuid_spelling_and_foreign_source_admission_are_preserved() {
    let budget = meter();
    let foreign = meter();
    let (mut driver, connection) = Driver::new(budget.clone()).unwrap();
    let id = connection.next_id().unwrap();
    match id {
        MessageId::String(text) => {
            let uuid = uuid::Uuid::parse_str(text.text()).unwrap();
            assert_eq!(uuid.get_version_num(), 4);
            assert_eq!(text.text(), uuid.hyphenated().to_string());
        }
        _ => panic!("original SDK uses UUID string ids"),
    }
    let foreign_envelope = call(&foreign, 1);
    match connection.request(foreign_envelope) {
        Err((ManagedError::ForeignResponse, envelope)) => drop(envelope),
        _ => panic!("foreign source was admitted"),
    }
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    let mut entries = Buffer::new(foreign.clone());
    entries
        .push(Entry::Message(notice(&budget)))
        .map_err(|(e, _)| e)
        .unwrap();
    assert!(matches!(
        driver.receive(Frame::batch(entries).unwrap()),
        Err(ManagedError::ForeignResponse)
    ));
    drop(driver);
    drop(connection);
    assert_eq!(budget.0.lock().unwrap().live, 0);
    assert_eq!(foreign.0.lock().unwrap().live, 0);
}

#[test]
fn prepared_request_publication_is_infallible_after_all_future_reservations_refuse() {
    let budget = meter();
    let (mut driver,connection) = Driver::new(budget.clone()).unwrap();
    let mut prepared = connection.prepare_request(call(&budget,41)).map_err(|(error,_)|error).unwrap().ordered();
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    let reservations = budget.0.lock().unwrap().reservations;
    budget.0.lock().unwrap().limit = 0;
    assert!(Pin::new(&mut prepared).poll(&mut cx()).is_pending());
    publish(&mut driver);
    assert_eq!(budget.0.lock().unwrap().reservations,reservations);
    drop(prepared);
    assert!(matches!(driver.poll_outgoing(&mut cx()),Poll::Ready(Some(Outgoing::Cancel(MessageId::Number(41))))));
}
#[test]
fn provisional_front_preserves_fifo_and_does_not_consume_an_early_response() {
    let budget = meter();
    let (mut driver,connection) = Driver::new(budget.clone()).unwrap();
    let mut first = connection.prepare_request(call(&budget,1)).map_err(|(error,_)|error).unwrap();
    let mut second = connection.prepare_request(call(&budget,2)).map_err(|(error,_)|error).unwrap();
    let ordinary = request(&connection,call(&budget,3));
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    let mut early = driver.receive(Frame::Single(result(&budget,1))).unwrap();
    assert!(next_dispatch(&mut early,&mut driver).is_none());
    assert!(Pin::new(&mut first).poll(&mut cx()).is_pending());
    match driver.poll_outgoing(&mut cx()) { Poll::Ready(Some(Outgoing::Message(envelope)))=>assert!(envelope.id()==Some(&MessageId::Number(1))),_=>panic!("first prepared position") }
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
    assert!(Pin::new(&mut second).poll(&mut cx()).is_pending());
    for id in [2,3] {
        match driver.poll_outgoing(&mut cx()) { Poll::Ready(Some(Outgoing::Message(envelope)))=>assert!(envelope.id()==Some(&MessageId::Number(id))),_=>panic!("original FIFO position") }
    }
    let mut actual = driver.receive(Frame::Single(result(&budget,1))).unwrap();
    assert!(next_dispatch(&mut actual,&mut driver).is_none());
    drop(response(&mut first));
    drop((second,ordinary));
}
#[test]
fn dropping_prepared_request_emits_no_wire_and_duplicate_preparation_preserves_waiter() {
    let budget = meter();
    let (mut driver,connection) = Driver::new(budget.clone()).unwrap();
    let mut prepared = connection.prepare_request(call(&budget,9)).map_err(|(error,_)|error).unwrap();
    assert!(connection.prepare_request(call(&budget,9)).is_err());
    assert!(Pin::new(&mut prepared).poll(&mut cx()).is_pending());
    publish(&mut driver);
    let mut actual = driver.receive(Frame::Single(result(&budget,9))).unwrap();
    assert!(next_dispatch(&mut actual,&mut driver).is_none());
    drop(response(&mut prepared));
    let never = connection.prepare_request(call(&budget,10)).map_err(|(error,_)|error).unwrap();
    drop(never);
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
}
#[test]
fn parked_request_front_still_allows_real_incoming_response_groups() {
    let budget = meter();
    let (mut driver,connection) = Driver::new(budget.clone()).unwrap();
    let _prepared = connection.prepare_request(call(&budget,1)).map_err(|(error,_)|error).unwrap();
    let mut incoming = driver.receive(Frame::Single(call(&budget,99))).unwrap();
    next_dispatch(&mut incoming,&mut driver).unwrap().unhandled();
    assert!(matches!(driver.poll_outgoing(&mut cx()),Poll::Ready(Some(Outgoing::Responses { .. }))));
    assert!(driver.poll_outgoing(&mut cx()).is_pending());
}
