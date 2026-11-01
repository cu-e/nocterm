//! Agent file transfers reach the transfer port only through approved, session-bound endpoints.
use super::*;
use gpui_kit::EntityId;
use nocterm_ai::TransferFiles;
use nocterm_session::{HostAddress, Target, testing::InMemoryFs};
use nocterm_transfers::{
    CollisionPolicy, Request, TransferEndpoint, TransferId, TransferService, TransferState,
};

fn host(name: &str) -> HostAddress {
    HostAddress::direct(Target {
        host: name.into(),
        port: 22,
        user: "user".into(),
    })
}

/// A background terminal served by its own remote host, for the chat to attach.
fn add_terminal(f: &Fixture, host_name: &str, cx: &mut TestAppContext) -> (Rc<Access>, EntityId) {
    let access = Access::new(None, Rc::default());
    access.serve_remote_files(host(host_name));
    let item = cx
        .update_window(f.handle, |_, window, cx| {
            let terminal = cx.new(|cx| FakeTerminal::new(access.clone(), cx));
            f.workspace.update(cx, |workspace, cx| {
                workspace.add_background_item(terminal, window, cx)
            })
        })
        .unwrap();
    (access, item)
}
fn id_of(thread: &Entity<AgentThread>, item: EntityId, cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread
                .resolved(cx)
                .into_iter()
                .find(|(_, entry, _)| entry.item == item)
                .map(|(id, _, _)| id)
                .expect("the terminal is attached")
        })
    })
}

struct Pair {
    f: Fixture,
    thread: Entity<AgentThread>,
    source: (Rc<Access>, String),
    destination: (Rc<Access>, String),
    third: String,
}
fn pair(cx: &mut TestAppContext) -> Pair {
    let f = fixture(cx);
    f.access.serve_remote_files(host("source"));
    let (destination, destination_item) = add_terminal(&f, "destination", cx);
    let (_, third_item) = add_terminal(&f, "third", cx);
    new_chat(&f, cx);
    let thread = current(&f, cx);
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Terminal(destination_item), cx);
            thread.attach(Attachment::Terminal(third_item), cx);
        })
    });
    let source = terminal_id(&thread, cx);
    Pair {
        source: (f.access.clone(), source),
        destination: (destination, id_of(&thread, destination_item, cx)),
        third: id_of(&thread, third_item, cx),
        f,
        thread,
    }
}

fn transfer(pair: &Pair, destination: &str) -> TerminalCall {
    transfer_between(&pair.source.1, destination)
}
fn transfer_between(source: &str, destination: &str) -> TerminalCall {
    TerminalCall::TransferFiles(TransferFiles {
        source_terminal_id: source.into(),
        sources: vec!["/srv/a".into(), "/srv/dir".into()],
        destination_terminal_id: destination.into(),
        destination: "/srv/in".into(),
        overwrite: true,
        yield_ms: Some(0),
    })
}
fn read_transfer(
    pair: &Pair,
    id: &str,
    cx: &mut TestAppContext,
) -> Result<Value, nocterm_ai::ToolFailure> {
    let call = TerminalCall::ReadTransfer(nocterm_ai::ReadTransfer {
        transfer_id: id.into(),
        yield_ms: Some(0),
    });
    result(submit(&pair.f, &pair.thread, call, true, cx))
}
fn submitted(pair: &Pair, payload: &Value) -> Request {
    let id: TransferId = payload["transfer_id"].as_str().unwrap().parse().unwrap();
    pair.f._directory.transfers.request(id).unwrap()
}
fn states(pair: &Pair) -> Vec<TransferState> {
    pair.f
        ._directory
        .transfers
        .snapshot()
        .into_iter()
        .map(|progress| progress.state)
        .collect()
}
fn decide(pair: &Pair, allow: bool, grant: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        pair.thread.update(cx, |thread, cx| {
            let token = thread.tool_cards()[0].token();
            thread.approve_tool(token, allow, grant, cx);
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_approved_transfer_submits_a_relay_between_the_captured_endpoints(cx: &mut TestAppContext) {
    let p = pair(cx);
    let payload = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    assert_eq!(payload["state"], "queued");
    assert_eq!(payload["direction"], "relay");
    let Request::Relay(relay) = submitted(&p, &payload) else {
        panic!("a remote to remote transfer is a relay");
    };
    assert_eq!(relay.sources, ["/srv/a", "/srv/dir"]);
    assert_eq!(relay.destination, "/srv/in");
    assert_eq!(relay.source_host, host("source"));
    assert_eq!(relay.host, host("destination"));
    assert_eq!(relay.collisions, CollisionPolicy::Replace);
}

#[gpui_kit::test]
fn a_local_source_uploads_and_a_denied_card_submits_nothing(cx: &mut TestAppContext) {
    let p = pair(cx);
    p.source.0.serve_local_files();
    let denied = submit(&p.f, &p.thread, transfer(&p, &p.destination.1), false, cx);
    assert!(
        states(&p).is_empty(),
        "nothing runs before the card is answered"
    );
    decide(&p, false, false, cx);
    assert!(result(denied).unwrap_err().to_string().contains("denied"));
    assert!(states(&p).is_empty());
    let payload = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    assert!(matches!(submitted(&p, &payload), Request::Upload(_)));
}

#[gpui_kit::test]
fn one_grant_covers_both_terminals_but_not_a_new_endpoint(cx: &mut TestAppContext) {
    let p = pair(cx);
    let third = p.third.clone();
    let first = submit(&p.f, &p.thread, transfer(&p, &p.destination.1), false, cx);
    decide(&p, true, true, cx);
    assert!(result(first).is_ok());
    let repeated = submit(&p.f, &p.thread, transfer(&p, &p.destination.1), false, cx);
    assert!(
        cx.update(|cx| p.thread.read(cx).tool_cards().is_empty()),
        "the granted pair needs no card"
    );
    assert!(result(repeated).is_ok());
    let _pending = submit(&p.f, &p.thread, transfer(&p, &third), false, cx);
    assert_eq!(cx.update(|cx| p.thread.read(cx).tool_cards().len()), 1);
}

#[gpui_kit::test]
fn grants_for_two_pairs_do_not_approve_a_transfer_across_them(cx: &mut TestAppContext) {
    let p = pair(cx);
    let (source, destination, third) =
        (p.source.1.clone(), p.destination.1.clone(), p.third.clone());
    let first = submit(
        &p.f,
        &p.thread,
        transfer_between(&source, &destination),
        false,
        cx,
    );
    decide(&p, true, true, cx);
    assert!(result(first).is_ok());
    let second = submit(
        &p.f,
        &p.thread,
        transfer_between(&destination, &third),
        false,
        cx,
    );
    assert_eq!(
        cx.update(|cx| p.thread.read(cx).tool_cards().len()),
        1,
        "the first pair's grant does not cover the destination's new partner"
    );
    decide(&p, true, true, cx);
    assert!(result(second).is_ok());
    let crossing = submit(
        &p.f,
        &p.thread,
        transfer_between(&source, &third),
        false,
        cx,
    );
    assert_eq!(
        cx.update(|cx| p.thread.read(cx).tool_cards().len()),
        1,
        "each terminal holds a grant, but never together"
    );
    decide(&p, false, false, cx);
    assert!(result(crossing).is_err());
    let reverse = submit(
        &p.f,
        &p.thread,
        transfer_between(&third, &destination),
        false,
        cx,
    );
    assert!(cx.update(|cx| p.thread.read(cx).tool_cards().is_empty()));
    assert!(result(reverse).is_ok());
}

#[gpui_kit::test]
fn a_source_reconnecting_between_card_and_approval_detaches_the_call(cx: &mut TestAppContext) {
    let p = pair(cx);
    let reply = submit(&p.f, &p.thread, transfer(&p, &p.destination.1), false, cx);
    p.source.0.replace_connection();
    decide(&p, true, false, cx);
    assert!(result(reply).unwrap_err().to_string().contains("detached"));
    assert!(states(&p).is_empty());
}

#[gpui_kit::test]
fn a_chat_reads_only_its_own_transfers(cx: &mut TestAppContext) {
    let p = pair(cx);
    let endpoint = |name: &str| TransferEndpoint::Remote {
        host: host(name),
        fs: Arc::new(InMemoryFs::default()),
    };
    let foreign = nocterm_transfers::TransferRequests::submit(
        &*p.f._directory.transfers,
        Request::between(
            endpoint("a"),
            vec!["/x".into()],
            endpoint("b"),
            "/y".into(),
            CollisionPolicy::Skip,
        )
        .unwrap(),
    )
    .unwrap();
    let refused = read_transfer(&p, &foreign.to_string(), cx).unwrap_err();
    assert!(refused.to_string().contains("not started by this chat"));
    let own = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    let id = own["transfer_id"].as_str().unwrap();
    assert_eq!(read_transfer(&p, id, cx).unwrap()["state"], "queued");
}

#[gpui_kit::test]
fn stopping_the_chat_cancels_its_active_transfers(cx: &mut TestAppContext) {
    let p = pair(cx);
    let _ = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    assert_eq!(states(&p), [TransferState::Queued]);
    cx.update(|cx| p.thread.update(cx, |thread, cx| thread.stop(cx)));
    cx.run_until_parked();
    assert_eq!(states(&p), [TransferState::Cancelled]);
}

#[gpui_kit::test]
fn replacing_an_endpoint_session_cancels_the_transfer(cx: &mut TestAppContext) {
    let p = pair(cx);
    let _ = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    p.source.0.replace_connection();
    cx.update(|cx| {
        p.thread
            .update(cx, |thread, cx| thread.revoke_executions(cx))
    });
    assert_eq!(states(&p), [TransferState::Cancelled]);
}

#[gpui_kit::test]
fn cancel_transfer_stops_an_own_transfer_and_reports_it(cx: &mut TestAppContext) {
    let p = pair(cx);
    let own = result(submit(
        &p.f,
        &p.thread,
        transfer(&p, &p.destination.1),
        true,
        cx,
    ))
    .unwrap();
    let call = TerminalCall::CancelTransfer(nocterm_ai::CancelTransfer {
        transfer_id: own["transfer_id"].as_str().unwrap().into(),
    });
    let report = result(submit(&p.f, &p.thread, call, true, cx)).unwrap();
    assert_eq!(report["state"], "cancelled");
}
