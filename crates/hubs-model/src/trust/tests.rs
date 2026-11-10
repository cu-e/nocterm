use super::*;

const PLAIN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn print() -> Fingerprint {
    Fingerprint::parse(PLAIN).unwrap()
}

#[test]
fn a_fingerprint_reads_hex_with_or_without_colons_in_any_case() {
    let colons = PLAIN
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).unwrap())
        .collect::<Vec<_>>()
        .join(":");
    assert_eq!(Fingerprint::parse(&colons).unwrap(), print());
    assert_eq!(Fingerprint::parse(&PLAIN.to_uppercase()).unwrap(), print());
    assert_eq!(
        Fingerprint::parse(&format!("  {PLAIN}\n")).unwrap(),
        print()
    );
    assert_eq!(print().as_bytes()[1], 0x11);
}

#[test]
fn a_fingerprint_of_the_wrong_length_or_with_other_characters_is_refused() {
    assert_eq!(
        Fingerprint::parse(&PLAIN[..62]),
        Err(FingerprintError::Length { found: 62 })
    );
    assert_eq!(
        Fingerprint::parse(&format!("{PLAIN}ab")),
        Err(FingerprintError::Length { found: 66 })
    );
    assert_eq!(
        Fingerprint::parse(&format!("{}zz", &PLAIN[..62])),
        Err(FingerprintError::NotHex)
    );
    assert_eq!(
        Fingerprint::parse(""),
        Err(FingerprintError::Length { found: 0 })
    );
}

#[test]
fn a_fingerprint_is_shown_with_colons_and_in_groups_of_four_digits() {
    let shown = print().to_string();
    assert!(shown.starts_with("00:11:22:33:44"));
    assert_eq!(shown.len(), 95);
    assert_eq!(Fingerprint::parse(&shown).unwrap(), print());
    let groups = print().groups();
    assert_eq!(groups[0], "00112233");
    assert_eq!(groups[7], "ccddeeff");
}

#[test]
fn a_typed_fingerprint_that_matches_pins_the_certificate() {
    let probe = ProbedHub {
        fingerprint: print(),
        publicly_trusted: false,
    };
    let decision = decide(&probe, Some(&print()));
    assert_eq!(decision, TrustDecision::Pinned(print()));
    assert_eq!(decision.settled(), Some(TrustPolicy::Pinned(print())));
    assert_eq!(decision.confirmed(), None);
}

#[test]
fn a_typed_fingerprint_that_differs_is_a_mismatch_even_for_a_public_certificate() {
    let other = Fingerprint::from_bytes([9; 32]);
    for publicly_trusted in [false, true] {
        let probe = ProbedHub {
            fingerprint: print(),
            publicly_trusted,
        };
        let decision = decide(&probe, Some(&other));
        assert_eq!(
            decision,
            TrustDecision::Mismatch {
                typed: other,
                presented: print()
            }
        );
        assert_eq!(decision.settled(), None);
        assert_eq!(decision.confirmed(), None, "a mismatch cannot be accepted");
    }
}

#[test]
fn a_public_certificate_without_a_typed_fingerprint_is_not_pinned() {
    let probe = ProbedHub {
        fingerprint: print(),
        publicly_trusted: true,
    };
    let decision = decide(&probe, None);
    assert_eq!(decision, TrustDecision::PublicCa);
    assert_eq!(decision.settled(), Some(TrustPolicy::PublicCa));
}

#[test]
fn a_self_signed_certificate_needs_one_confirmation_before_it_is_pinned() {
    let probe = ProbedHub {
        fingerprint: print(),
        publicly_trusted: false,
    };
    let decision = decide(&probe, None);
    assert_eq!(decision, TrustDecision::ConfirmFirstUse(print()));
    assert_eq!(decision.settled(), None);
    assert_eq!(decision.confirmed(), Some(TrustPolicy::Pinned(print())));
}
