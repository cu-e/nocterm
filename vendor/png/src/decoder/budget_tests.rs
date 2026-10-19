use super::*;
use std::{cell::Cell, io::Cursor, rc::Rc};

#[derive(Clone)]
struct Account(Rc<State>);
struct State {
    live: Cell<usize>,
    limit: usize,
    refused: Cell<bool>,
    reject_bytes: Cell<Option<usize>>,
}
impl Account {
    fn new(limit: usize) -> Self {
        Self(Rc::new(State {
            live: Cell::new(0),
            limit,
            refused: Cell::new(false),
            reject_bytes: Cell::new(None),
        }))
    }
}
impl AllocationBudget for Account {
    fn admit(&self, bytes: usize) -> Result<(), AllocationRefused> {
        let next = self
            .0
            .live
            .get()
            .checked_add(bytes)
            .ok_or(AllocationRefused)?;
        if self.0.refused.get() || next > self.0.limit || self.0.reject_bytes.get() == Some(bytes) {
            self.0.refused.set(true);
            return Err(AllocationRefused);
        }
        self.0.live.set(next);
        Ok(())
    }
    fn retire(&self, bytes: usize) {
        self.0
            .live
            .set(self.0.live.get().checked_sub(bytes).unwrap());
    }
}
fn encoded(extra: impl FnOnce(&mut crate::Writer<&mut Vec<u8>>)) -> Vec<u8> {
    let mut result = Vec::new();
    {
        let mut encoder = crate::Encoder::new(&mut result, 2, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        extra(&mut writer);
        writer
            .write_image_data(&[1, 2, 3, 255, 4, 5, 6, 255])
            .unwrap();
    }
    result
}
#[test]
fn managed_original_parser_pays_metadata_and_releases_all_native_buffers() {
    let bytes = encoded(|writer| {
        writer
            .write_chunk(crate::chunk::tEXt, b"key\0v\xff")
            .unwrap();
        writer
            .write_chunk(crate::chunk::zTXt, b"key\0\0compressed-preserved")
            .unwrap();
        writer
            .write_chunk(crate::chunk::iTXt, b"key\0\0\0en\0translated\0text")
            .unwrap();
    });
    let account = Account::new(1024 * 1024);
    let mut decoder =
        Decoder::new_with_budget(Cursor::new(&bytes), Limits::default(), account.clone()).unwrap();
    decoder.set_ignore_text_chunk(false);
    decoder.set_transformations(Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    assert_eq!(reader.info().uncompressed_latin1_text[0].text, "vÿ");
    assert_eq!(reader.info().compressed_latin1_text.len(), 1);
    assert_eq!(reader.info().utf8_text[0].translated_keyword, "translated");
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut output).unwrap();
    assert_eq!(output, [1, 2, 3, 255, 4, 5, 6, 255]);
    assert!(account.0.live.get() > 0);
    drop(reader);
    assert_eq!(account.0.live.get(), 0);
}
#[test]
fn optional_icc_refusal_is_sticky_while_invalid_icc_keeps_original_acceptance() {
    let profile = vec![7; 40000];
    let compressed = fdeflate::compress_to_vec(&profile);
    let mut iccp = b"profile\0\0".to_vec();
    iccp.extend_from_slice(&compressed);
    let bytes = encoded(|writer| writer.write_chunk(crate::chunk::iCCP, &iccp).unwrap());
    let enough = Account::new(1024 * 1024);
    let decoder =
        Decoder::new_with_budget(Cursor::new(&bytes), Limits::default(), enough.clone()).unwrap();
    let reader = decoder.read_info().unwrap();
    assert_eq!(
        &**reader.info().icc_profile.as_ref().unwrap(),
        profile.as_slice()
    );
    drop(reader);
    assert_eq!(enough.0.live.get(), 0);
    let limited = Account::new(35000);
    if let Ok(decoder) =
        Decoder::new_with_budget(Cursor::new(&bytes), Limits::default(), limited.clone())
    {
        drop(decoder.read_info());
    }
    assert!(limited.0.refused.get());
    assert_eq!(limited.0.live.get(), 0);
    let invalid = encoded(|writer| {
        writer
            .write_chunk(crate::chunk::iCCP, b"profile\0\0invalid zlib")
            .unwrap()
    });
    let enough = Account::new(1024 * 1024);
    let decoder =
        Decoder::new_with_budget(Cursor::new(&invalid), Limits::default(), enough.clone()).unwrap();
    let mut reader = decoder.read_info().unwrap();
    assert!(reader.info().icc_profile.is_none());
    reader.next_frame(&mut [0; 8]).unwrap();
    assert!(!enough.0.refused.get());
    drop(reader);
    assert_eq!(enough.0.live.get(), 0);
}
#[test]
fn refusal_during_metadata_vector_intake_drops_payload_before_its_receipt() {
    let bytes = encoded(|writer| {
        writer
            .write_chunk(crate::chunk::tEXt, b"key\0value")
            .unwrap()
    });
    let account = Account::new(1024 * 1024);
    account
        .0
        .reject_bytes
        .set(Some(std::mem::size_of::<crate::text_metadata::TEXtChunk>()));
    let mut decoder =
        Decoder::new_with_budget(Cursor::new(&bytes), Limits::default(), account.clone()).unwrap();
    decoder.set_ignore_text_chunk(false);
    assert!(decoder.read_info().is_err());
    assert!(account.0.refused.get());
    assert_eq!(account.0.live.get(), 0);
}
