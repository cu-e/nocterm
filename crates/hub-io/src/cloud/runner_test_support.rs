//! Native source fixtures, not a network client or authentication fake.
use nocterm_cloud_model::{identity::*, journal::JournalCatalogBinding};
use nocterm_hub_client::cloud::RunnerSession;
use nocterm_hub_model::cloud::{CloudNamespace, CloudTextId, runner_auth::RunnerProofSubject};
use nocterm_hub_proto::v1::CloudRunnerTokens;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn binding(account_key: u8) -> JournalCatalogBinding {
    JournalCatalogBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes([account_key; 32]),
            ),
        ),
        RunnerId::from_bytes([3; 16]),
        RunnerKeyEpoch::new(4).unwrap(),
    )
}
pub(super) fn subject() -> RunnerProofSubject {
    RunnerProofSubject {
        raw_hub_id: CloudTextId::new("raw-hub").unwrap(),
        namespace: CloudNamespace {
            hub_instance_id: CloudTextId::new("hub").unwrap(),
            account_id: CloudTextId::new("account").unwrap(),
        },
        runner_id: [3; 16],
        proof_key: [4; 32],
    }
}
pub(super) fn session(subject: &RunnerProofSubject, generation: &str) -> RunnerSession {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    RunnerSession::from_response(
        CloudRunnerTokens {
            account_id: subject.namespace.account_id.as_str().to_owned(),
            runner_id: subject.runner_id.to_vec(),
            access_token: format!("access-{generation}"),
            access_expires_at_ms: now + 60_000,
            refresh_token: format!("refresh-{generation}"),
            refresh_expires_at_ms: now + 600_000,
            capability_version: 1,
        },
        subject,
    )
    .unwrap()
}
