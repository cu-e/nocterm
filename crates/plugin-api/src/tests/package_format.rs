use alloc::{string::String, vec::Vec};

use crate::{MANIFEST_SECTION, PackageKind, SectionError, kind, manifest_section};

const HEADER: [u8; 8] = [0, b'a', b's', b'm', 1, 0, 0, 0];

fn leb(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            bytes.push(byte);
            return bytes;
        }
        bytes.push(byte | 0x80);
    }
}

fn section(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::from([id]);
    bytes.extend(leb(payload.len() as u32));
    bytes.extend_from_slice(payload);
    bytes
}

fn custom(name: &str, data: &[u8]) -> Vec<u8> {
    let mut payload = leb(name.len() as u32);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(data);
    section(0, &payload)
}

fn module(sections: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    sections.iter().for_each(|s| bytes.extend_from_slice(s));
    bytes
}

#[test]
fn the_kind_comes_from_the_magic_not_from_a_name() {
    assert_eq!(kind(&HEADER), PackageKind::Module);
    assert_eq!(kind(b"api = \"0.1\""), PackageKind::Manifest);
    assert_eq!(kind(b""), PackageKind::Manifest);
    assert_eq!(kind(b"\0as"), PackageKind::Manifest);
    assert_eq!(PackageKind::Manifest.extension(), "toml");
    assert_eq!(PackageKind::Module.extension(), "wasm");
}

#[test]
fn the_manifest_section_is_found_between_other_sections() {
    let bytes = module(&[
        custom("name", b"ignored"),
        section(1, &[0, 0]),
        custom(MANIFEST_SECTION, b"api = \"0.1\""),
        section(10, &[1, 2, 3]),
    ]);
    assert_eq!(manifest_section(&bytes), Ok(&b"api = \"0.1\""[..]));
}

#[test]
fn a_large_manifest_uses_a_multi_byte_size() {
    let text: String = "a".repeat(40_000);
    let bytes = module(&[custom(MANIFEST_SECTION, text.as_bytes())]);
    assert_eq!(manifest_section(&bytes).map(<[u8]>::len), Ok(40_000));
}

#[test]
fn a_file_that_is_not_a_version_1_module_is_refused() {
    assert_eq!(manifest_section(b""), Err(SectionError::Header));
    assert_eq!(
        manifest_section(b"api = \"0.1\""),
        Err(SectionError::Header)
    );
    assert_eq!(
        manifest_section(&[0, b'a', b's', b'm', 2, 0, 0, 0]),
        Err(SectionError::Header)
    );
    assert_eq!(manifest_section(&HEADER[..6]), Err(SectionError::Header));
}

#[test]
fn a_module_without_the_section_says_so() {
    assert_eq!(manifest_section(&HEADER), Err(SectionError::Missing));
    let other = module(&[custom("producers", b"x"), section(1, &[0])]);
    assert_eq!(manifest_section(&other), Err(SectionError::Missing));
}

#[test]
fn two_manifest_sections_are_refused() {
    let bytes = module(&[
        custom(MANIFEST_SECTION, b"one"),
        custom(MANIFEST_SECTION, b"two"),
    ]);
    assert_eq!(manifest_section(&bytes), Err(SectionError::Duplicate));
}

#[test]
fn a_size_past_the_end_is_truncation_not_a_read() {
    let mut bytes = module(&[custom(MANIFEST_SECTION, b"abcdef")]);
    bytes.truncate(bytes.len() - 1);
    assert_eq!(manifest_section(&bytes), Err(SectionError::Truncated));
    let mut huge = HEADER.to_vec();
    huge.extend([0, 0xff, 0xff, 0xff, 0xff, 0x0f]);
    assert_eq!(manifest_section(&huge), Err(SectionError::Truncated));
    let mut name = HEADER.to_vec();
    name.extend(section(0, &[50, b'a']));
    assert_eq!(manifest_section(&name), Err(SectionError::Truncated));
    let mut id_only = HEADER.to_vec();
    id_only.push(0);
    assert_eq!(manifest_section(&id_only), Err(SectionError::Truncated));
}

#[test]
fn a_size_that_overflows_32_bits_is_a_leb_error() {
    let mut six = HEADER.to_vec();
    six.extend([1, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
    assert_eq!(manifest_section(&six), Err(SectionError::Leb));
    let mut high = HEADER.to_vec();
    high.extend([1, 0xff, 0xff, 0xff, 0xff, 0x1f]);
    assert_eq!(manifest_section(&high), Err(SectionError::Leb));
    let mut name = HEADER.to_vec();
    let mut payload = Vec::from([0x80, 0x80, 0x80, 0x80, 0x80]);
    payload.push(0);
    name.extend(section(0, &payload));
    assert_eq!(manifest_section(&name), Err(SectionError::Leb));
}

#[test]
fn any_prefix_of_a_module_is_refused_or_read_without_panicking() {
    let bytes = module(&[
        custom("name", b"ignored"),
        custom(MANIFEST_SECTION, b"api = \"0.1\""),
    ]);
    for end in 0..bytes.len() {
        let _ = manifest_section(&bytes[..end]);
    }
    assert!(manifest_section(&bytes).is_ok());
}
