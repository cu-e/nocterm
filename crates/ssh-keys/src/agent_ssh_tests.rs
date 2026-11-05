use crate::testing::ca::{
    CertificateError, CertificateSpec, IssuedCertificate, MAX_VALIDITY_MS, UserCa, fingerprint_of,
};
use ssh_key::{HashAlg, certificate::CertType};

fn ca() -> UserCa {
    UserCa::from_seed([8; 32]).unwrap()
}

fn spec() -> CertificateSpec {
    CertificateSpec {
        principal: "nocterm-agent-abc".into(),
        subject_key: [6; 32],
        serial: 7,
        valid_after_ms: 1_700_000_000_000,
        valid_before_ms: 1_700_000_000_000 + MAX_VALIDITY_MS,
        source_addresses: vec!["203.0.113.7/32".into(), "2001:db8::1".into()],
    }
}

#[test]
fn an_issued_certificate_is_a_narrow_user_certificate_signed_by_the_ca() {
    let issued = ca().issue(&spec(), [1; 32]).unwrap();
    let cert = IssuedCertificate::parse(issued.openssh()).unwrap();
    assert_eq!(cert.cert_type(), CertType::User);
    assert_eq!(cert.valid_principals(), ["nocterm-agent-abc"]);
    assert_eq!(cert.serial(), 7);
    assert_eq!(
        cert.critical_options()
            .get("source-address")
            .map(String::as_str),
        Some("203.0.113.7/32,2001:db8::1")
    );
    let extensions: Vec<_> = cert.extensions().keys().map(String::as_str).collect();
    assert_eq!(
        extensions,
        ["permit-pty"],
        "no port, agent or X11 forwarding"
    );
    assert_eq!(cert.valid_before() - cert.valid_after(), 24 * 60 * 60 + 60);
    let fingerprint = ca().public_text().fingerprint().to_owned();
    assert_eq!(
        cert.signature_key()
            .fingerprint(HashAlg::Sha256)
            .to_string(),
        fingerprint
    );
    cert.validate_at(
        1_700_000_100,
        [&cert.signature_key().fingerprint(HashAlg::Sha256)],
    )
    .unwrap();
}

#[test]
fn a_certificate_made_for_another_ca_does_not_validate_against_this_one() {
    let other = UserCa::from_seed([9; 32]).unwrap();
    let issued = other.issue(&spec(), [1; 32]).unwrap();
    let cert = IssuedCertificate::parse(issued.openssh()).unwrap();
    let ours = ssh_key::PublicKey::from_openssh(ca().public_text().as_str()).unwrap();
    assert!(
        cert.validate_at(1_700_000_100, [&ours.fingerprint(HashAlg::Sha256)])
            .is_err()
    );
}

#[test]
fn unsafe_requests_are_refused_before_signing() {
    let refused = |change: fn(&mut CertificateSpec)| {
        let mut s = spec();
        change(&mut s);
        ca().issue(&s, [1; 32]).unwrap_err()
    };
    assert_eq!(
        refused(|s| s.principal = "root".into()),
        CertificateError::Principal
    );
    assert_eq!(
        refused(|s| s.principal = "nocterm-agent-a\"b".into()),
        CertificateError::Principal
    );
    assert_eq!(
        refused(|s| s.valid_before_ms += 1),
        CertificateError::Validity
    );
    assert_eq!(
        refused(|s| s.valid_before_ms = s.valid_after_ms),
        CertificateError::Validity
    );
    assert_eq!(
        refused(|s| s.source_addresses.clear()),
        CertificateError::SourceAddress
    );
    assert_eq!(
        refused(|s| s.source_addresses = vec!["a b".into()]),
        CertificateError::SourceAddress
    );
}

#[test]
fn the_ca_is_reproduced_from_its_seed_and_its_fingerprint_matches_the_public_key() {
    let first = ca();
    let again = UserCa::from_seed(*first.seed()).unwrap();
    assert_eq!(first.public_key(), again.public_key());
    assert_eq!(fingerprint_of(&first.public_key()), first.fingerprint());
    assert_ne!(
        first.public_key(),
        UserCa::from_seed([1; 32]).unwrap().public_key()
    );
}

#[test]
fn adding_a_line_keeps_every_other_line_and_is_idempotent() {
    use crate::authorized_keys::with_line;
    let ours = "cert-authority,principals=\"nocterm-agent-a\" ssh-ed25519 AAAA nocterm";
    let file = "ssh-ed25519 AAAA me\n# note\nssh-rsa BBBB you";
    let once = with_line(file, ours);
    assert_eq!(
        once,
        format!("ssh-ed25519 AAAA me\n# note\nssh-rsa BBBB you\n{ours}\n")
    );
    assert_eq!(with_line(&once, ours), once);
    assert_eq!(with_line("", ours), format!("{ours}\n"));
}
