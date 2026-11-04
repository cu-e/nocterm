//! A chat's SSH binding, certificates and revocation through the coordinator (ADR 0137).
use super::*;
use nocterm_cloud_model::{
    payload::OpaqueBytes,
    ssh_access::{
        BoundTarget, CertificateFacts, ChatSshCredentials, ChatSshKeys, SshAccessError,
        SshAccessEvent, SshAccessPhase, SshBindingOffer, SshCertificateOffer, SshCertificateReader,
        SshCertificateRequest, SshEndpoint, SshReach, SshServerKey,
    },
};
use std::sync::Mutex;

const HOUR: u64 = 3_600_000;

pub(super) type SshAccessInputs = (
    nocterm_cloud_model::ssh_access::SshReach,
    Arc<dyn nocterm_cloud_model::ssh_access::SshCertificateReader>,
);
/// Adds chat SSH access over the fixture's journal when the test asks for it.
pub(super) fn with_ssh_access(
    execution: CoordinatorExecution,
    journal: &Arc<nocterm_cloud_runner::DurableRunJournal>,
    inputs: Option<SshAccessInputs>,
) -> (
    CoordinatorExecution,
    Option<Arc<nocterm_cloud_runner::coordinator::CoordinatorSsh>>,
) {
    let Some((reach, certificates)) = inputs else {
        return (execution, None);
    };
    let ssh = Arc::new(nocterm_cloud_runner::coordinator::CoordinatorSsh::new(
        journal.clone(),
        certificates,
        reach,
    ));
    (execution.with_ssh(ssh.clone()), Some(ssh))
}
impl Fixture {
    /// A fixture whose coordinator admits chat SSH bindings with this reach and reader.
    pub(super) fn with_ssh(
        reach: nocterm_cloud_model::ssh_access::SshReach,
        certificates: Arc<dyn nocterm_cloud_model::ssh_access::SshCertificateReader>,
    ) -> Self {
        Self::build(
            SupervisorLimits::default(),
            CommandReceiveBudget::new(IntakeLimits::default()),
            Arc::new(ScriptedPublicationTransport::default()),
            None,
            Some((reach, certificates)),
        )
    }
}

/// Reads every certificate as the facts the test set.
#[derive(Default)]
struct ScriptedCertificates(Mutex<Option<CertificateFacts>>);
impl SshCertificateReader for ScriptedCertificates {
    fn read(&self, _: &[u8], _: &[u8; 32]) -> Result<CertificateFacts, SshAccessError> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or(SshAccessError::Invalid)
    }
}

fn endpoint() -> SshEndpoint {
    SshEndpoint {
        host: "db.example".into(),
        port: 22,
        user: "deploy".into(),
    }
}
fn grant(epoch: u64) -> GrantReference {
    GrantReference::new(
        GrantClass::SshBinding,
        GrantId::from_bytes([70; 16]),
        GrantEpoch::new(epoch).unwrap(),
    )
}
fn binding(id: u8, control: ChatControlRevision) -> ReceivedCommand {
    let offer = SshBindingOffer::new(
        grant(1),
        Timestamp::from_millis(500),
        Timestamp::from_millis(500 + 24 * HOUR),
        [71; 32],
        vec![BoundTarget {
            server: SshServerKey::from_bytes([72; 16]),
            endpoint: endpoint(),
            host_key: fp(73),
            principal: "nocterm-agent-test".into(),
        }],
        OpaqueBytes::new(vec![74; 32]).unwrap(),
    )
    .unwrap();
    command(id, control, None, CommandBody::SshBinding(Box::new(offer)))
}
fn ssh_events(fixture: &Fixture) -> Vec<SshAccessEvent> {
    fixture
        .events
        .events()
        .into_iter()
        .filter_map(|event| match event {
            CoordinatorEvent::SshAccess(event) => Some(*event),
            _ => None,
        })
        .collect()
}
fn last_request(fixture: &Fixture) -> SshCertificateRequest {
    ssh_events(fixture)
        .into_iter()
        .rev()
        .find_map(|event| match event {
            SshAccessEvent::Requested(request) => Some(*request),
            SshAccessEvent::State(_) => None,
        })
        .expect("a certificate request")
}
fn last_phase(fixture: &Fixture) -> SshAccessPhase {
    ssh_events(fixture)
        .into_iter()
        .rev()
        .find_map(|event| match event {
            SshAccessEvent::State(report) => Some(report.access),
            SshAccessEvent::Requested(_) => None,
        })
        .expect("a state event")
}
/// A bound chat whose first certificate (valid 8 hours from now) was accepted.
async fn certified() -> (Fixture, Arc<ScriptedCertificates>) {
    let certificates = Arc::new(ScriptedCertificates::default());
    let fixture = Fixture::with_ssh(
        SshReach::new(vec![(endpoint(), Some(fp(73)))], vec!["203.0.113.7".into()]).unwrap(),
        certificates.clone(),
    );
    fixture
        .actor
        .accept(binding(1, ChatControlRevision::default()))
        .await
        .unwrap();
    assert_eq!(last_phase(&fixture), SshAccessPhase::Pending);
    let request = last_request(&fixture);
    *certificates.0.lock().unwrap() = Some(CertificateFacts {
        subject_key: request.subject_key,
        principals: vec!["nocterm-agent-test".into()],
        valid_after: Timestamp::from_millis(1000),
        valid_before: Timestamp::from_millis(1000 + 8 * HOUR),
    });
    let answer = SshCertificateOffer::new(
        grant(1),
        request.request,
        OpaqueBytes::new(vec![75; 64]).unwrap(),
    )
    .unwrap();
    fixture
        .actor
        .accept(command(
            2,
            fixture.control().await,
            None,
            CommandBody::SshCertificate(Box::new(answer)),
        ))
        .await
        .unwrap();
    (fixture, certificates)
}

#[tokio::test]
async fn a_bound_chat_asks_for_a_certificate_and_lends_it_to_sessions_once_answered() {
    let (fixture, _) = certified().await;
    assert!(matches!(
        last_phase(&fixture),
        SshAccessPhase::Active { .. }
    ));
    let ssh = fixture.ssh.clone().unwrap();
    let now = Timestamp::from_millis(2000);
    let credential = ssh
        .credential(&stream(), &endpoint(), &fp(73), now)
        .unwrap();
    assert_eq!(credential.principal, "nocterm-agent-test");
    assert_eq!(credential.certificate.as_bytes(), &[75; 64]);
    assert_eq!(
        credential.seed.public_key(),
        last_request(&fixture).subject_key
    );
    assert!(
        ssh.credential(&stream(), &endpoint(), &fp(99), now)
            .is_none()
    );
}

#[tokio::test]
async fn the_next_certificate_is_requested_at_eighty_percent_with_the_same_key() {
    let (fixture, _) = certified().await;
    let first = last_request(&fixture);
    fixture.timer.set(1000 + 8 * HOUR / 100 * 80 - 1);
    fixture.actor.renew_ssh().await.unwrap();
    assert_eq!(last_request(&fixture).request, first.request);
    fixture.timer.set(1000 + 8 * HOUR / 100 * 80);
    fixture.actor.renew_ssh().await.unwrap();
    let renewal = last_request(&fixture);
    assert_ne!(renewal.request, first.request);
    assert_eq!(renewal.subject_key, first.subject_key);
}

#[tokio::test]
async fn unbinding_revokes_access_and_forgets_the_key() {
    let (fixture, _) = certified().await;
    fixture
        .actor
        .accept(command(
            3,
            fixture.control().await,
            None,
            CommandBody::RevokeGrant(grant(1)),
        ))
        .await
        .unwrap();
    assert_eq!(last_phase(&fixture), SshAccessPhase::Revoked);
    let ssh = fixture.ssh.clone().unwrap();
    assert!(
        ssh.credential(
            &stream(),
            &endpoint(),
            &fp(73),
            Timestamp::from_millis(2000)
        )
        .is_none()
    );
    let stored = fixture.journal.ssh_access(stream()).await.unwrap().unwrap();
    assert!(stored.seed.is_none() && stored.access.binding.is_none());
}

#[tokio::test]
async fn a_certificate_for_another_request_is_refused_and_the_binding_stays_pending() {
    let certificates = Arc::new(ScriptedCertificates::default());
    let fixture = Fixture::with_ssh(
        SshReach::new(vec![(endpoint(), None)], vec!["203.0.113.7".into()]).unwrap(),
        certificates.clone(),
    );
    fixture
        .actor
        .accept(binding(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let answer =
        SshCertificateOffer::new(grant(1), [0; 16], OpaqueBytes::new(vec![75; 64]).unwrap())
            .unwrap();
    *certificates.0.lock().unwrap() = Some(CertificateFacts {
        subject_key: last_request(&fixture).subject_key,
        principals: vec!["nocterm-agent-test".into()],
        valid_after: Timestamp::from_millis(1000),
        valid_before: Timestamp::from_millis(1000 + HOUR),
    });
    let refused = fixture
        .actor
        .accept(command(
            2,
            fixture.control().await,
            None,
            CommandBody::SshCertificate(Box::new(answer)),
        ))
        .await;
    assert!(refused.is_err());
    assert_eq!(last_phase(&fixture), SshAccessPhase::Pending);
}

#[tokio::test]
async fn a_runner_without_egress_refuses_the_binding_and_keeps_no_key() {
    let fixture = Fixture::with_ssh(
        SshReach::new(vec![(endpoint(), None)], Vec::new()).unwrap(),
        Arc::new(ScriptedCertificates::default()),
    );
    fixture
        .actor
        .accept(binding(1, ChatControlRevision::default()))
        .await
        .unwrap();
    assert!(matches!(last_phase(&fixture), SshAccessPhase::Refused(_)));
    let stored = fixture.journal.ssh_access(stream()).await.unwrap().unwrap();
    assert!(stored.seed.is_none());
}
