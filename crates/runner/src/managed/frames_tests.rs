use super::*;
use crate::managed::testing::{chat, token, tokens};
use nocterm_hub_model::agents::ChatAccessFrame;
use std::io::Cursor;

fn issue(byte: u8, text: &str) -> Vec<u8> {
    ChatAccessFrame::Issue {
        chat: [byte; 16],
        token: token(text),
    }
    .encode()
    .to_vec()
}
fn revoke(byte: u8) -> Vec<u8> {
    ChatAccessFrame::Revoke { chat: [byte; 16] }
        .encode()
        .to_vec()
}
/// Hands out one byte per read, as a slow pipe would.
struct Trickle(Cursor<Vec<u8>>);
impl Read for Trickle {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let take = 1.min(buffer.len());
        self.0.read(&mut buffer[..take])
    }
}
fn boot_frame(json: &serde_json::Value) -> Vec<u8> {
    let body = json.to_string().into_bytes();
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}
fn boot_document() -> serde_json::Value {
    serde_json::json!({
        "version": 2, "instance": "inst1", "agent": "agent1", "lease": "lease1",
        "name": "Scribe", "instructions": "be brief",
        "provider": {"kind": "openai_chat", "base_url": "http://10.0.0.2:8080/v1",
            "models": ["gpt-x"],
            "pricing": {"maximum_request_bytes": 65536, "maximum_output_tokens": 2048,
                "maximum_call_cost_micros": 50000}},
        "secret_names": [], "allowed_hosts": [], "ssh_targets": []
    })
}

#[test]
fn the_boot_frame_is_a_length_prefixed_v2_document_and_the_frames_follow_it() {
    let mut bytes = boot_frame(&boot_document());
    bytes.extend(issue(1, "abc"));
    let mut input = Cursor::new(bytes);
    assert!(read_boot_v2(&mut input).is_ok());
    let mut rest = Vec::new();
    input.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, issue(1, "abc"));
}

#[test]
fn an_oversized_empty_or_truncated_boot_frame_is_refused() {
    let too_long = u32::try_from(MAX_BOOT_BYTES + 1).unwrap().to_be_bytes();
    assert!(matches!(
        read_boot_v2(&mut &too_long[..]),
        Err(RunnerError::Value("boot document size"))
    ));
    assert!(read_boot_v2(&mut &0u32.to_be_bytes()[..]).is_err());
    let mut cut = boot_frame(&boot_document());
    cut.truncate(cut.len() - 5);
    assert!(matches!(
        read_boot_v2(&mut &cut[..]),
        Err(RunnerError::Boot(_))
    ));
}

#[tokio::test]
async fn issue_and_revoke_frames_update_the_stores_even_when_the_pipe_trickles() {
    let store = tokens();
    let mut bytes = issue(1, "one");
    bytes.extend(issue(2, "two"));
    bytes.extend(revoke(1));
    let end = pump(Trickle(Cursor::new(bytes)), &store);
    assert!(matches!(end, ControlEnd::Eof));
    // End of input revokes the rest as well.
    assert!(store.lease(chat(2)).is_err());
}

#[tokio::test]
async fn tokens_stay_usable_while_the_channel_is_open() {
    let store = Arc::new(tokens());
    let mut bytes = issue(1, "one");
    bytes.extend(issue(2, "two"));
    bytes.extend(revoke(1));
    // A reader that blocks after the frames, as a live pipe does.
    let (gate, hold) = std::sync::mpsc::channel::<()>();
    struct Held(Cursor<Vec<u8>>, std::sync::mpsc::Receiver<()>);
    impl Read for Held {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            match self.0.read(buffer)? {
                0 => {
                    let _ = self.1.recv();
                    Ok(0)
                }
                count => Ok(count),
            }
        }
    }
    let closed = spawn_reader(Held(Cursor::new(bytes), hold), store.clone()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while store.lease(chat(2)).is_err() {
        assert!(std::time::Instant::now() < deadline, "issue never applied");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(store.lease(chat(1)).is_err(), "revoked chat has no token");
    drop(gate);
    assert!(matches!(closed.await, Ok(ControlEnd::Eof)));
    assert!(store.lease(chat(2)).is_err());
}

#[tokio::test]
async fn an_oversized_unknown_or_trailing_frame_closes_the_channel_and_revokes_everything() {
    let mut oversize = u32::MAX.to_be_bytes().to_vec();
    oversize.extend([0; 8]);
    let mut version = issue(2, "two");
    version[4] = 9;
    let mut trailing = revoke(2);
    trailing.push(0);
    // Growing the length prefix makes the extra byte part of the body.
    trailing[3] += 1;
    for bad in [oversize, version, trailing] {
        let store = tokens();
        let mut bytes = issue(1, "one");
        bytes.extend(bad);
        bytes.extend(issue(3, "three"));
        let end = pump(Cursor::new(bytes), &store);
        assert!(
            matches!(end, ControlEnd::Refused(RunnerError::ControlChannel(_))),
            "{end:?}"
        );
        assert!(store.lease(chat(1)).is_err());
        assert!(store.lease(chat(3)).is_err(), "nothing after the bad frame");
        assert!(
            store.issue(chat(4), &token("four")).is_err(),
            "closed for good"
        );
    }
}

#[tokio::test]
async fn a_repeated_issue_replaces_the_token_and_a_revoke_of_an_unknown_chat_is_harmless() {
    let store = tokens();
    store.issue(chat(1), &token("a")).unwrap();
    let first = store.lease(chat(1)).unwrap();
    store.issue(chat(1), &token("b")).unwrap();
    assert!(!first.is_available(), "the replaced token ends");
    assert!(store.lease(chat(1)).unwrap().is_available());
    store.revoke(chat(9));
}
