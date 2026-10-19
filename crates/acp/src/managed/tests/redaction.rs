//! The exact original secret/Unicode regex chain is the diagnostic byte oracle.
use super::fixture;
use crate::managed::redaction::DiagnosticRedactor;
use agent_client_protocol::managed::Source;
#[test]
fn original_diagnostic_redaction_order_unicode_captures_and_growth_are_preserved() {
    let redactor = DiagnosticRedactor::new().unwrap();
    for raw in [
        "ordinary é Ω harmless",
        "DB_PASSWORD=hunter2 api_key: 'k-123' client_secret=\"abc\" Authorization: Bearer abc.def postgres://app:s3cret@db:5432/x \"github_token\": \"value\"",
        "-----BEGIN RSA PRIVATE KEY-----\nΩ secret\n-----END RSA PRIVATE KEY----- and sk-proj-abcdefghijklmnop",
        "-----BEGIN PRIVATE KEY BLOCK-----\nunterminated Ω secret",
        "éauthorization: Bearer foo authorization: Bearer bar İAUTHORIZATION: Token a é API_KEY=x",
        "https://üser:password@example.test ghp_abcdefghijklmnopqrst xoxb-1234567890-abcdefgh",
        "secret a secretsecret longersecret tail",
    ] {
        for secret_text in [
            vec![],
            vec!["longersecret", "secret"],
            vec!["a", "secret"],
            vec![""],
        ] {
            let fixture = fixture();
            let source = Source::copy(raw, fixture.budget.clone()).unwrap();
            let secrets = secret_text
                .iter()
                .map(|secret| Source::copy(secret, fixture.budget.clone()).unwrap())
                .collect::<Vec<_>>();
            let output = redactor.scrub(source, &secrets, &fixture.budget).unwrap();
            let mut expected = raw.to_string();
            for secret in &secret_text {
                expected = expected.replace(secret, "[REDACTED]");
            }
            expected = nocterm_ai::redact::redact(&expected);
            assert_eq!(output.text(), expected, "{raw}: {secret_text:?}");
        }
    }
}
#[test]
fn redacted_owned_diagnostic_keeps_exact_original_source_credit_after_close() {
    let redactor = DiagnosticRedactor::new().unwrap();
    let fixture = fixture();
    let source = Source::copy("TOKEN=original-secret", fixture.budget.clone()).unwrap();
    let output = redactor.owned(source, &[], &fixture.budget).unwrap();
    assert_eq!(output.as_text(), "TOKEN=[REDACTED]");
    let paid = fixture.budget.ledger().live_requested_bytes();
    drop(fixture._lease);
    assert_eq!(output.as_text(), "TOKEN=[REDACTED]");
    assert!(fixture.budget.ledger().live_requested_bytes() >= paid);
    drop(output);
    assert!(fixture.budget.ledger().live_requested_bytes() < paid);
}
