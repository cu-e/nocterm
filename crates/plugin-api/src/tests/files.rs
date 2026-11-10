//! The files of a plugin's terminal: paths the host asks about, answers it accepts, and the
//! base64 the bytes travel in.

use alloc::{string::String, vec, vec::Vec};

use crate::{
    LimitError, base64,
    capability::{
        files::{
            FileAnswer, FileEntry, FileFailure, FileOp, FileReply, MAX_FILE_BYTES,
            MAX_FILE_ENTRIES, check_path,
        },
        net::Request,
    },
};

#[test]
fn base64_round_trips_every_length_and_refuses_what_is_not_base64() {
    let bytes: Vec<u8> = (0..=255u8).collect();
    for len in 0..40 {
        let text = base64::encode(&bytes[..len]);
        assert_eq!(base64::decoded_len(&text), Some(len));
        assert_eq!(base64::decode(&text).as_deref(), Some(&bytes[..len]));
    }
    assert_eq!(base64::encode(b"Man"), "TWFu");
    assert_eq!(base64::encode(b"Ma"), "TWE=");
    for wrong in ["TWF", "TW=u", "T===", "TWE=TWFu", "TW!u"] {
        assert_eq!(base64::decode(wrong), None, "{wrong}");
    }
}

#[test]
fn the_host_asks_only_about_plain_absolute_paths() {
    for good in ["/", "/plugins", "/plugins/", "/a b/c.yml"] {
        assert_eq!(check_path(good), Ok(()), "{good}");
    }
    for bad in [
        "", "plugins", "/a/../b", "/a/./b", "//etc", "/a//b", "/a\0b",
    ] {
        assert!(check_path(bad).is_err(), "{bad}");
    }
    let rename = FileOp::Rename {
        from: String::from("/a"),
        to: String::from("/../b"),
    };
    assert!(rename.check().is_err());
}

fn reply(answer: FileAnswer) -> FileReply {
    FileReply {
        terminal: String::from("srv-1"),
        request: 3,
        answer,
    }
}

fn entry(name: &str) -> FileEntry {
    FileEntry {
        name: String::from(name),
        directory: false,
        size: Some(1),
        modified: None,
    }
}

#[test]
fn answers_hold_plain_names_and_bounded_contents() {
    assert_eq!(
        reply(FileAnswer::Listed(vec![entry("a.yml")])).check(),
        Ok(())
    );
    for bad in ["", ".", "..", "a/b", "a\nb", "exe.\u{202e}txt"] {
        assert!(
            reply(FileAnswer::Listed(vec![entry(bad)])).check().is_err(),
            "{bad:?}"
        );
    }
    let many = vec![entry("a"); MAX_FILE_ENTRIES + 1];
    assert!(reply(FileAnswer::Listed(many)).check().is_err());
    let full = base64::encode(&vec![7u8; MAX_FILE_BYTES]);
    assert_eq!(reply(FileAnswer::Content(full)).check(), Ok(()));
    let over = base64::encode(&vec![7u8; MAX_FILE_BYTES + 1]);
    assert!(reply(FileAnswer::Content(over)).check().is_err());
    assert_eq!(
        reply(FileAnswer::Content(String::from("abc"))).check(),
        Err(LimitError::Syntax { field: "content" })
    );
    assert_eq!(
        reply(FileAnswer::Failed(FileFailure::NotFound)).check(),
        Ok(())
    );
}

#[test]
fn a_bytes_request_carries_base64() {
    let request = |body: &str| Request {
        method: crate::capability::net::Method::Post,
        path: String::from("/write"),
        body: Some(String::from(body)),
        bytes: true,
    };
    assert_eq!(request("TWFu").check(), Ok(()));
    assert!(request("not base64").check().is_err());
}
