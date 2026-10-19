//! A paid native tail preserves original suffix and lossy Unicode semantics.
use super::fixture;
use crate::managed::stderr::Tail;
#[test]
fn native_tail_matches_original_suffix_and_lossy_utf8_across_wraps() {
    let fixture = fixture();
    let mut tail = Tail::new(fixture.budget.clone());
    let mut original = Vec::new();
    for bytes in [
        b"prefix".as_slice(),
        &vec![b'x'; 64 * 1024 - 4],
        " Ω terminal ".as_bytes(),
        &[0xe2, 0x82],
        &[0xff, 0x61],
        &vec![b'z'; 70 * 1024],
    ] {
        tail.push(bytes).unwrap();
        original.extend_from_slice(bytes);
        let excess = original.len().saturating_sub(64 * 1024);
        original.drain(..excess);
        let decoded = tail.decoded().unwrap();
        assert_eq!(decoded.text(), String::from_utf8_lossy(&original));
    }
}
