use super::*;
use crate::identity::{GrantEpoch, GrantId};

const KEY: [u8; 32] = [7; 32];
const HOUR: u64 = 60 * 60 * 1000;
const T0: u64 = 1_000_000;

fn at(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}
fn grant(id: u8, epoch: u64) -> GrantReference {
    GrantReference::new(
        GrantClass::SshBinding,
        GrantId::from_bytes([id; 16]),
        GrantEpoch::new(epoch).unwrap(),
    )
}
fn endpoint(host: &str) -> SshEndpoint {
    SshEndpoint {
        host: host.into(),
        port: 22,
        user: "deploy".into(),
    }
}
fn target(server: u8, host: &str) -> BoundTarget {
    BoundTarget {
        server: SshServerKey::from_bytes([server; 16]),
        endpoint: endpoint(host),
        host_key: Fingerprint::from_bytes([server; 32]),
        principal: format!("nocterm-agent-ca-{server}"),
        access: Some(access_of(server)),
    }
}
/// The session access the adapter derives for server `server` of the binding.
fn access_of(server: u8) -> BoundAccess {
    BoundAccess {
        target: TargetBinding::new(
            Fingerprint::from_bytes([server; 32]),
            Fingerprint::from_bytes([server ^ 0x40; 32]),
            crate::identity::NamespaceId::new("deploy".to_owned()).unwrap(),
            Fingerprint::from_bytes([server ^ 0x80; 32]),
        ),
        authentication: AuthenticationBinding::new(
            Fingerprint::from_bytes([9; 32]),
            Fingerprint::from_bytes([server ^ 0xc0; 32]),
        ),
    }
}
/// Hosts `a`, `b`, … are servers 1, 2, … with host keys `[1; 32]`, `[2; 32]`, ….
fn host_key(host: &str) -> Fingerprint {
    Fingerprint::from_bytes([host.as_bytes()[0] - b'a' + 1; 32])
}
fn offer(epoch: u64, targets: Vec<BoundTarget>) -> SshBindingOffer {
    SshBindingOffer::new(
        grant(1, epoch),
        at(T0),
        at(T0 + 7 * 24 * HOUR),
        [9; 32],
        targets,
        OpaqueBytes::new(vec![1, 2, 3]).unwrap(),
    )
    .unwrap()
}
fn reach(hosts: &[&str], egress: &[&str]) -> SshReach {
    SshReach::new(
        hosts
            .iter()
            .map(|host| (endpoint(host), host_key(host)))
            .collect(),
        egress.iter().map(|address| (*address).to_owned()).collect(),
    )
    .unwrap()
}
fn requested(change: &SshAccessChange) -> &SshCertificateRequest {
    change
        .events
        .iter()
        .find_map(|event| match event {
            SshAccessEvent::Requested(request) => Some(request.as_ref()),
            SshAccessEvent::State(_) => None,
        })
        .expect("a certificate request")
}
fn phase(change: &SshAccessChange) -> SshAccessPhase {
    change
        .events
        .iter()
        .find_map(|event| match event {
            SshAccessEvent::State(report) => Some(report.access),
            SshAccessEvent::Requested(_) => None,
        })
        .expect("a state event")
}
fn facts(request: &SshCertificateRequest, valid_before: u64) -> CertificateFacts {
    CertificateFacts {
        subject_key: request.subject_key,
        principals: request
            .servers
            .iter()
            .map(|server| format!("nocterm-agent-ca-{}", server.as_bytes()[0]))
            .collect(),
        valid_after: at(T0 + 1),
        valid_before: at(valid_before),
    }
}
fn certificate(request: &SshCertificateRequest) -> SshCertificateOffer {
    SshCertificateOffer::new(
        grant(1, 1),
        request.request,
        OpaqueBytes::new(vec![4; 64]).unwrap(),
    )
    .unwrap()
}
/// A chat bound to `a` and `b` on a runner that reaches only `a`, certified for 8 hours.
fn active() -> (ChatSshAccess, SshReach) {
    let reach = reach(&["a"], &["203.0.113.7"]);
    let mut access = ChatSshAccess::new();
    let bound = access
        .bind(
            offer(1, vec![target(1, "a"), target(2, "b")]),
            &reach,
            &KEY,
            at(T0 + 1),
            [3; 16],
        )
        .unwrap();
    let request = requested(&bound).clone();
    let change = access
        .certify(
            &certificate(&request),
            &facts(&request, T0 + 1 + 8 * HOUR),
            &KEY,
            at(T0 + 2),
        )
        .unwrap();
    assert!(matches!(phase(&change), SshAccessPhase::Active { .. }));
    (access, reach)
}

#[test]
fn a_binding_admits_only_reachable_servers_and_requests_a_certificate_from_its_egress() {
    let mut access = ChatSshAccess::new();
    let change = access
        .bind(
            offer(1, vec![target(1, "a"), target(2, "b")]),
            &reach(&["a"], &["203.0.113.7"]),
            &KEY,
            at(T0 + 1),
            [3; 16],
        )
        .unwrap();
    assert_eq!(phase(&change), SshAccessPhase::Pending);
    assert_eq!(change.key, KeyChange::Keep);
    let request = requested(&change);
    assert_eq!(request.servers, vec![SshServerKey::from_bytes([1; 16])]);
    assert_eq!(request.source_addresses, vec!["203.0.113.7".to_owned()]);
    assert_eq!(request.subject_key, KEY);
    assert_eq!(
        request.not_after,
        at(T0 + 1 + DEFAULT_CERTIFICATE_VALIDITY_MS)
    );
    assert_eq!(request.binding.as_bytes(), &[1, 2, 3]);
}

#[test]
fn a_runner_without_egress_or_reachable_servers_refuses_the_binding_and_needs_no_key() {
    for (reach, refusal) in [
        (reach(&["a"], &[]), SshBindingRefusal::NoSourceAddress),
        (
            reach(&["elsewhere"], &["10.0.0.1"]),
            SshBindingRefusal::NoReachableServer,
        ),
    ] {
        let mut access = ChatSshAccess::new();
        let change = access
            .bind(
                offer(1, vec![target(1, "a")]),
                &reach,
                &KEY,
                at(T0 + 1),
                [3; 16],
            )
            .unwrap();
        assert_eq!(phase(&change), SshAccessPhase::Refused(refusal));
        assert_eq!(change.events.len(), 1);
        assert_eq!(change.key, KeyChange::Forget);
        assert!(!access.needs_key());
    }
}

#[test]
fn a_configured_target_with_another_host_key_is_unreachable() {
    let configured = SshReach::new(
        vec![(endpoint("a"), Fingerprint::from_bytes([8; 32]))],
        vec!["10.0.0.1".into()],
    )
    .unwrap();
    assert_eq!(
        configured.admits(&target(1, "a")),
        SshServerReach::Unreachable
    );
    assert_eq!(
        reach(&["a"], &["10.0.0.1"]).admits(&target(1, "a")),
        SshServerReach::Admitted
    );
}

#[test]
fn an_older_or_equal_binding_epoch_is_refused_even_after_an_unbind() {
    let reach = reach(&["a"], &["10.0.0.1"]);
    let mut access = ChatSshAccess::new();
    access
        .bind(
            offer(2, vec![target(1, "a")]),
            &reach,
            &KEY,
            at(T0 + 1),
            [3; 16],
        )
        .unwrap();
    for epoch in [1, 2] {
        assert_eq!(
            access.bind(
                offer(epoch, vec![target(1, "a")]),
                &reach,
                &KEY,
                at(T0 + 1),
                [3; 16]
            ),
            Err(SshAccessError::Stale)
        );
    }
    access.revoke(grant(1, 2)).unwrap();
    assert_eq!(
        access.bind(
            offer(2, vec![target(1, "a")]),
            &reach,
            &KEY,
            at(T0 + 1),
            [3; 16]
        ),
        Err(SshAccessError::Stale)
    );
    assert!(
        access
            .bind(
                offer(3, vec![target(1, "a")]),
                &reach,
                &KEY,
                at(T0 + 1),
                [3; 16]
            )
            .is_ok()
    );
}

#[test]
fn a_certificate_is_refused_for_another_request_key_principal_set_or_time() {
    let reach = reach(&["a"], &["10.0.0.1"]);
    let mut access = ChatSshAccess::new();
    let bound = access
        .bind(
            offer(1, vec![target(1, "a")]),
            &reach,
            &KEY,
            at(T0 + 1),
            [3; 16],
        )
        .unwrap();
    let request = requested(&bound).clone();
    let good = facts(&request, T0 + HOUR);
    let mut other_request = request.clone();
    other_request.request = [4; 16];
    let cases = [
        (
            certificate(&other_request),
            good.clone(),
            SshAccessError::UnknownRequest,
        ),
        (
            certificate(&request),
            CertificateFacts {
                subject_key: [8; 32],
                ..good.clone()
            },
            SshAccessError::SubjectKey,
        ),
        (
            certificate(&request),
            CertificateFacts {
                principals: vec!["nocterm-agent-ca-2".into()],
                ..good.clone()
            },
            SshAccessError::Principals,
        ),
        (
            certificate(&request),
            facts(&request, T0 + 9 * HOUR),
            SshAccessError::Validity,
        ),
    ];
    for (offer, facts, error) in cases {
        assert_eq!(access.certify(&offer, &facts, &KEY, at(T0 + 2)), Err(error));
    }
    let other_grant = SshCertificateOffer::new(
        grant(2, 1),
        request.request,
        OpaqueBytes::new(vec![1]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        access.certify(&other_grant, &good, &KEY, at(T0 + 2)),
        Err(SshAccessError::OtherGrant)
    );
}

#[test]
fn an_active_certificate_signs_in_to_its_bound_servers_only() {
    let (access, _) = active();
    let key = Fingerprint::from_bytes([1; 32]);
    let credential = access.credential(&endpoint("a"), &key, at(T0 + 3)).unwrap();
    assert_eq!(credential.principal, "nocterm-agent-ca-1");
    assert!(
        access
            .credential(
                &endpoint("b"),
                &Fingerprint::from_bytes([2; 32]),
                at(T0 + 3)
            )
            .is_none()
    );
    assert!(
        access
            .credential(
                &endpoint("a"),
                &Fingerprint::from_bytes([2; 32]),
                at(T0 + 3)
            )
            .is_none()
    );
    assert!(
        access
            .credential(&endpoint("a"), &key, at(T0 + 1 + 8 * HOUR))
            .is_none()
    );
}

#[test]
fn a_run_selects_an_admitted_bound_server_only_by_its_exact_session_access() {
    let (access, _) = active();
    let one = access_of(1);
    let selected = access
        .select(&one.target, &one.authentication, at(T0 + 3))
        .unwrap();
    assert_eq!(selected.target.endpoint, endpoint("a"));
    assert_eq!(selected.credential.unwrap().principal, "nocterm-agent-ca-1");
    // Server 2 is bound but this runner cannot reach it.
    let two = access_of(2);
    assert!(
        access
            .select(&two.target, &two.authentication, at(T0 + 3))
            .is_none()
    );
    // Another credential revision (an older or newer binding epoch) selects nothing.
    let stale = AuthenticationBinding::new(
        Fingerprint::from_bytes([9; 32]),
        Fingerprint::from_bytes([0xee; 32]),
    );
    assert!(access.select(&one.target, &stale, at(T0 + 3)).is_none());
    // After the certificate lapsed the server is still selected and waits for a device.
    let lapsed = access
        .select(&one.target, &one.authentication, at(T0 + 1 + 8 * HOUR))
        .unwrap();
    assert!(lapsed.credential.is_none());
    assert!(
        access
            .select(&one.target, &one.authentication, at(T0 + 7 * 24 * HOUR))
            .is_none()
    );
}

#[test]
fn the_next_certificate_is_requested_at_eighty_percent_and_an_unanswered_one_lapses() {
    let (mut access, reach) = active();
    let renew = T0 + 1 + 8 * HOUR / 100 * 80;
    assert_eq!(access.next_due(), Some(at(renew)));
    assert!(
        access
            .tick(&reach, &KEY, at(renew - 1), [5; 16])
            .events
            .is_empty()
    );
    let renewal = access.tick(&reach, &KEY, at(renew), [5; 16]);
    assert_eq!(requested(&renewal).request, [5; 16]);
    assert!(
        access
            .tick(&reach, &KEY, at(renew + 1), [6; 16])
            .events
            .is_empty()
    );
    let lapsed = access.tick(&reach, &KEY, at(T0 + 1 + 8 * HOUR), [6; 16]);
    assert_eq!(phase(&lapsed), SshAccessPhase::Pending);
    assert_eq!(lapsed.key, KeyChange::Keep);
}

#[test]
fn revocation_and_the_end_of_the_window_forget_the_key() {
    let (mut revoked, _) = active();
    assert_eq!(revoked.revoke(grant(2, 1)), Err(SshAccessError::OtherGrant));
    let change = revoked.revoke(grant(1, 1)).unwrap();
    assert_eq!(phase(&change), SshAccessPhase::Revoked);
    assert_eq!(change.key, KeyChange::Forget);
    assert!(
        revoked
            .credential(
                &endpoint("a"),
                &Fingerprint::from_bytes([1; 32]),
                at(T0 + 3)
            )
            .is_none()
    );

    let (mut expired, reach) = active();
    let change = expired.tick(&reach, &KEY, at(T0 + 7 * 24 * HOUR), [5; 16]);
    assert_eq!(phase(&change), SshAccessPhase::Expired);
    assert_eq!(change.key, KeyChange::Forget);
    assert_eq!(expired.next_due(), None);
}

#[test]
fn stored_parts_restore_the_same_access_and_inconsistent_ones_are_refused() {
    let (access, _) = active();
    let restored = ChatSshAccess::restore(access.parts().clone()).unwrap();
    assert_eq!(restored, access);
    let mut broken = access.parts().clone();
    if let Some(binding) = broken.binding.as_mut() {
        binding.refused = Some(SshBindingRefusal::Invalid);
    }
    assert_eq!(ChatSshAccess::restore(broken), Err(SshAccessError::Invalid));
}

#[test]
fn malformed_offers_and_egress_are_refused() {
    let signed = || OpaqueBytes::new(vec![1]).unwrap();
    assert!(
        SshBindingOffer::new(
            grant(1, 1),
            at(T0),
            at(T0 + 31 * 24 * HOUR),
            [9; 32],
            vec![target(1, "a")],
            signed()
        )
        .is_err()
    );
    assert!(
        SshBindingOffer::new(
            grant(1, 1),
            at(T0),
            at(T0 + HOUR),
            [9; 32],
            vec![target(1, "a"), target(1, "b")],
            signed()
        )
        .is_err()
    );
    let provider = GrantReference::new(
        GrantClass::Provider,
        GrantId::from_bytes([1; 16]),
        GrantEpoch::new(1).unwrap(),
    );
    assert!(
        SshBindingOffer::new(
            provider,
            at(T0),
            at(T0 + HOUR),
            [9; 32],
            vec![target(1, "a")],
            signed()
        )
        .is_err()
    );
    assert!(SshReach::new(Vec::new(), vec!["host.example".into()]).is_err());
    assert!(
        SshCertificateOffer::new(
            grant(1, 1),
            [1; 16],
            OpaqueBytes::new(vec![0; MAX_CERTIFICATE_BYTES + 1]).unwrap()
        )
        .is_err()
    );
}
