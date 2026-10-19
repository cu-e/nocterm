//! Exact historical14+17MiB bytes through current held literal-string capability.
//! Experiment requires explicit frozen fixture; no full fixture Vec/Value wrapper.
use super::*;
use crate::loading::literal_reader::exact::decode_authenticated_string;
use crate::loading::{
    held_file::{HeldFile, ReadStamp, plan::AuthenticatedPlan},
    json_fields::Key,
    json_frames::{Element, FrameError, Member, RootFrames},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};
const FIXTURE_SHA: &str = "47c48d09d6dd6bb9e3cb82d1f5aac206fb0c448f4ab193ff321cf4d1d4323eab";
fn digest(path: &Path) -> [u8; 32] {
    let mut source = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut block = [0; 4096];
    loop {
        let read = source.read(&mut block).unwrap();
        if read == 0 {
            break;
        }
        hash.update(&block[..read]);
    }
    hash.finalize().into()
}
fn member(source: &mut dyn Read, start: usize, key: Key) -> Result<Member, FrameError> {
    let mut frames = RootFrames::new_literal(io::BufReader::with_capacity(256, source), start)?;
    let mut result = None;
    let RootFrames::Object(object) = &mut frames else {
        return Err(FrameError::Punctuation);
    };
    while let Some(field) = object.next_member()? {
        if field.kind() == key {
            if result.is_some() {
                return Err(FrameError::Punctuation);
            }
            result = Some(field);
        }
    }
    frames.finish()?;
    result.ok_or(FrameError::Punctuation)
}
fn entries(source: &mut dyn Read, start: usize) -> Result<[Element; 2], FrameError> {
    let mut frames = RootFrames::new_literal(io::BufReader::with_capacity(256, source), start)?;
    let RootFrames::Array(array) = &mut frames else {
        return Err(FrameError::Punctuation);
    };
    let first = array.next_element()?.ok_or(FrameError::Truncated)?;
    let second = array.next_element()?.ok_or(FrameError::Truncated)?;
    if array.next_element()?.is_some() {
        return Err(FrameError::Punctuation);
    }
    frames.finish()?;
    Ok([first, second])
}
fn text_plans(held: &mut HeldFile, stamp: &ReadStamp) -> [AuthenticatedPlan<Member>; 2] {
    let root = held
        .plan_pass(stamp)
        .scan(|reader| member(reader, 0, Key::Entries))
        .unwrap()
        .transpose()
        .unwrap();
    let start = root.payload().value().start();
    let pair = root
        .derive(
            held,
            |field| field.native_span(),
            |reader| entries(reader, start),
        )
        .unwrap()
        .transpose()
        .unwrap();
    let start = pair.payload()[0].value().start();
    let first = pair
        .derive(
            held,
            |rows| rows[0].native_span(),
            |reader| member(reader, start, Key::Value),
        )
        .unwrap()
        .transpose()
        .unwrap();
    let start = pair.payload()[1].value().start();
    let second = pair
        .derive(
            held,
            |rows| rows[1].native_span(),
            |reader| member(reader, start, Key::Value),
        )
        .unwrap()
        .transpose()
        .unwrap();
    let start = second.payload().value().start();
    let resource = second
        .derive(
            held,
            |field| field.native_span(),
            |reader| member(reader, start, Key::Resource),
        )
        .unwrap()
        .transpose()
        .unwrap();
    let start = resource.payload().value().start();
    let text = resource
        .derive(
            held,
            |field| field.native_span(),
            |reader| member(reader, start, Key::Text),
        )
        .unwrap()
        .transpose()
        .unwrap();
    [first, text]
}
#[test]
fn controlled_literal_decode_preserves_original_14_and_17_mib_text_acceptance() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let path = std::env::var_os("NOCTERM_NEUTRAL_LARGE_FIXTURE").expect(
        "explicit authenticated historical14+17 fixture required; missing is not a skipped pass",
    );
    let path = Path::new(&path);
    // The streaming digest helper's4096byte block is gone before any decode.
    let before = digest(path);
    let actual_sha = before
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual_sha, FIXTURE_SHA);
    drop(actual_sha);
    let ledger = DecodeLedger::new(&authority, nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut held = HeldFile::open(path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let bounds = stamp.bounds();
    assert_eq!(stamp.encoded_bytes(), 32506067);
    assert_eq!(
        (
            bounds.nodes,
            bounds.decoded_text_bytes,
            bounds.structural_bytes
        ),
        (29, 32505989, 65026826)
    );
    let [first, text] = text_plans(&mut held, &stamp);
    let previous = decode_authenticated_string(
        &first,
        &mut held,
        |field| field.native_span(),
        &ledger,
    )
    .unwrap()
    .expect("historically accepted14MiB text must survive current controlled literal construction");
    let borrowed = previous.as_owned().as_value().as_str().unwrap();
    assert_eq!(borrowed.len(), 14 * 1024 * 1024);
    assert!(borrowed.bytes().all(|byte| byte == b'a'));
    let identity = borrowed.as_ptr();
    let result =
        decode_authenticated_string(&text, &mut held, |field| field.native_span(), &ledger)
            .unwrap();
    let embedded=result.expect("historically accepted17MiB embedded text must survive while original14MiB text remains owned under unchanged64MiB construction ledger");
    let borrowed = embedded.as_owned().as_value().as_str().unwrap();
    assert_eq!(borrowed.len(), 17 * 1024 * 1024);
    assert!(borrowed.bytes().all(|byte| byte == b'r'));
    assert_eq!(
        previous.as_owned().as_value().as_str().unwrap().as_ptr(),
        identity
    );
    assert!(ledger.peak_requested_bytes() <= nocterm_ai::history::MAX_RESIDENT_BYTES);
    assert_eq!(digest(path), before);
}

#[test]
fn general_nested_literal_keeps_both_historical_large_texts_in_the_same_final_tree() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let fixture = std::env::var_os("NOCTERM_NEUTRAL_LARGE_FIXTURE")
        .expect("explicit authenticated31MiB fixture is required");
    let path = Path::new(&fixture);
    let before = digest(path);
    let actual_sha = before
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual_sha, FIXTURE_SHA);
    drop(actual_sha);
    let ledger = DecodeLedger::new(&authority, nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut held = HeldFile::open(path).unwrap();
    let stamp = held.preflight_literal(&ledger).unwrap();
    let plan = held
        .plan_pass(&stamp)
        .scan(|reader| member(reader, 0, Key::Entries))
        .unwrap()
        .transpose()
        .unwrap();
    let value = decode_authenticated_json(&plan, &mut held, |field| field.native_span(), &ledger)
        .unwrap()
        .expect(
            "same genuine nested literal route must retain both previously accepted large texts",
        );
    let borrowed = value.as_owned().as_value();
    let first = borrowed[0]["value"].as_str().unwrap();
    let second = borrowed[1]["value"]["resource"]["text"].as_str().unwrap();
    assert_eq!(first.len(), 14 * 1024 * 1024);
    assert!(first.bytes().all(|byte| byte == b'a'));
    assert_eq!(second.len(), 17 * 1024 * 1024);
    assert!(second.bytes().all(|byte| byte == b'r'));
    assert!(value.as_owned().receipt().matches(value.as_owned()));
    assert!(ledger.peak_requested_bytes() <= nocterm_ai::history::MAX_RESIDENT_BYTES);
    assert_eq!(digest(path), before);
}
