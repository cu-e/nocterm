use super::*;
use crate::testing::LoginProofFixture;
#[test]
fn host_proof_must_sign_the_exact_initial_session_and_pinned_key() {
    let fixture = LoginProofFixture::default();
    let proof = fixture.host_proof([3; 32]);
    proof.verify(&fixture.host_key(), &[3; 32]).unwrap();
    assert_eq!(
        proof.verify(&fixture.host_key(), &[4; 32]),
        Err(LoginProofError::HostSignature)
    );
    assert_eq!(
        proof.verify(&fixture.user_key(), &[3; 32]),
        Err(LoginProofError::Mismatch)
    );
}
#[test]
fn malformed_truncated_and_appended_actions_are_never_signable() {
    let fixture = LoginProofFixture::default();
    let login = fixture.login("alice", [3; 32]);
    for end in 0..login.payload().len() {
        assert!(
            UserauthLogin::parse(login.payload()[..end].to_vec()).is_err(),
            "{end}"
        );
    }
    let mut payload = login.payload().to_vec();
    payload.push(0);
    assert!(UserauthLogin::parse(payload).is_err());
    let mut payload = login.payload().to_vec();
    payload[36] = 51;
    assert_eq!(
        UserauthLogin::parse(payload),
        Err(LoginProofError::Unsupported)
    );
    assert!(UserauthLogin::parse(vec![0; MAX_LOGIN_PAYLOAD + 1]).is_err());
}
#[test]
fn a_checked_login_signature_cannot_authenticate_another_action() {
    let fixture = LoginProofFixture::default();
    let login = fixture.login("alice", [3; 32]);
    let key = LoginSigningKey::import(&fixture.user_secret(), &fixture.user_key()).unwrap();
    let signature = key.sign_login(&login).unwrap();
    signature.verify(&login).unwrap();
    assert_eq!(
        signature.verify(&fixture.login("bob", [3; 32])),
        Err(LoginProofError::Mismatch)
    );
    assert_eq!(
        signature.verify(&fixture.login("alice", [4; 32])),
        Err(LoginProofError::Mismatch)
    );
    assert!(LoginSigningKey::import(&fixture.user_secret(), &fixture.host_key()).is_err());
}
#[test]
fn wrong_service_method_unsigned_flag_algorithm_and_host_suffix_are_rejected() {
    let fixture = LoginProofFixture::default();
    let login = fixture.login("alice", [3; 32]);
    for text in [b"ssh-connection".as_slice(), b"publickey", b"ssh-ed25519"] {
        let mut payload = login.payload().to_vec();
        let start = payload.windows(text.len()).position(|w| w == text).unwrap();
        payload[start] ^= 1;
        assert!(UserauthLogin::parse(payload).is_err());
    }
    let mut payload = login.payload().to_vec();
    let flag = payload.windows(9).position(|w| w == b"publickey").unwrap() + 9;
    payload[flag] = 0;
    assert!(UserauthLogin::parse(payload).is_err());
    let proof = fixture.host_proof([3; 32]);
    let mut sig = proof.signature_blob().to_vec();
    sig.push(0);
    assert!(InitialHostProof::new(proof.host_key_blob().to_vec(), sig).is_err());
}
