use super::*;
#[test]
fn hostile_initial_stream_is_bounded_without_a_panic() {
    let mut observation = Observation::default();
    assert!(observation.feed(&vec![0; MAX_INITIAL + 1]).is_err());
    for size in [0u32, 1, 5, u32::MAX] {
        let mut o = Observation::default();
        o.feed(b"SSH-2.0-test\r\n").unwrap();
        let mut bytes = size.to_be_bytes().to_vec();
        bytes.push(4);
        assert!(o.feed(&bytes).is_err());
    }
}
#[test]
fn initial_identification_survives_every_fragment_boundary() {
    let mut observation = Observation::default();
    for byte in b"banner\r\nSSH-2.0-test\r\n" {
        observation.feed(&[*byte]).unwrap();
    }
    assert!(observation.proof().is_err());
    assert!(observation.feed(&[0, 0, 0, 6, 5, 21, 0, 0, 0, 0]).is_err());
}
