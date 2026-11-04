//! The transitions of one chat's SSH access.
use super::{
    BindingParts, CertificateFacts, DEFAULT_CERTIFICATE_VALIDITY_MS, IssuedCertificate,
    MAX_BOUND_SERVERS, MAX_CERTIFICATE_VALIDITY_MS, OpenRequest, RENEW_AT_PERCENT, SshAccessError,
    SshAccessEvent, SshAccessParts, SshAccessPhase, SshAccessReport, SshBindingOffer,
    SshBindingRefusal, SshCertificateOffer, SshCertificateRequest, SshEndpoint, SshReach,
    SshServerKey, SshServerReach,
};
use crate::{
    coordinator::GrantReference,
    identity::{Fingerprint, Timestamp},
    payload::OpaqueBytes,
};

/// A device may backdate a certificate by a minute (clock skew).
const BACKDATE_MS: u64 = 60_000;
/// Replays are refused for this many distinct binding grant ids.
const MAX_KNOWN_GRANTS: usize = 16;

/// Whether the runner keeps the chat's subject key after a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyChange {
    Keep,
    /// No binding needs the key any more: its custodian deletes it.
    Forget,
}

/// The events a transition publishes in the chat and what happens to the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshAccessChange {
    pub events: Vec<SshAccessEvent>,
    pub key: KeyChange,
}
impl SshAccessChange {
    fn keep(events: Vec<SshAccessEvent>) -> Self {
        Self {
            events,
            key: KeyChange::Keep,
        }
    }
    fn forget(events: Vec<SshAccessEvent>) -> Self {
        Self {
            events,
            key: KeyChange::Forget,
        }
    }
}

/// What a session needs to sign in to one bound server with the chat's key.
#[derive(Clone, Copy, Debug)]
pub struct SshCredential<'a> {
    pub server: SshServerKey,
    pub principal: &'a str,
    pub certificate: &'a OpaqueBytes,
    pub valid_before: Timestamp,
}

/// One chat's binding, its open certificate request and its active certificate.
/// Time and request ids are supplied; the subject's public key is passed in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatSshAccess {
    parts: SshAccessParts,
}
impl ChatSshAccess {
    pub fn new() -> Self {
        Self::default()
    }
    /// Restores stored parts, refusing any that no transition could have produced.
    pub fn restore(parts: SshAccessParts) -> Result<Self, SshAccessError> {
        if parts.epochs.len() > MAX_KNOWN_GRANTS {
            return Err(SshAccessError::Invalid);
        }
        if let Some(binding) = &parts.binding {
            let offer = &binding.offer;
            let bound = |servers: &[SshServerKey]| {
                !servers.is_empty()
                    && servers.len() <= MAX_BOUND_SERVERS
                    && servers.iter().all(|server| offer.target(*server).is_some())
            };
            if binding.reach.len() != offer.targets().len()
                || binding.request.as_ref().is_some_and(|r| !bound(&r.servers))
                || binding
                    .certificate
                    .as_ref()
                    .is_some_and(|c| !bound(&c.servers) || c.valid_before <= c.valid_after)
                || (binding.refused.is_some()
                    && (binding.request.is_some() || binding.certificate.is_some()))
            {
                return Err(SshAccessError::Invalid);
            }
        }
        Ok(Self { parts })
    }
    pub fn parts(&self) -> &SshAccessParts {
        &self.parts
    }
    /// Whether a binding still needs the chat's subject key.
    pub fn needs_key(&self) -> bool {
        self.parts
            .binding
            .as_ref()
            .is_some_and(|binding| binding.refused.is_none())
    }
    /// Admits a newer binding: selects the servers this runner may reach, refuses one it
    /// cannot serve, and otherwise asks for the first certificate.
    pub fn bind(
        &mut self,
        offer: SshBindingOffer,
        reach: &SshReach,
        subject: &[u8; 32],
        now: Timestamp,
        request: [u8; 16],
    ) -> Result<SshAccessChange, SshAccessError> {
        if now < offer.issued() || now >= offer.expires() {
            return Err(SshAccessError::Invalid);
        }
        self.note_epoch(offer.grant())?;
        let reach_list: Vec<_> = offer
            .targets()
            .iter()
            .map(|target| (target.server, reach.admits(target)))
            .collect();
        let refused = if reach.egress().is_empty() {
            Some(SshBindingRefusal::NoSourceAddress)
        } else if !reach_list
            .iter()
            .any(|(_, reach)| *reach == SshServerReach::Admitted)
        {
            Some(SshBindingRefusal::NoReachableServer)
        } else {
            None
        };
        self.parts.binding = Some(BindingParts {
            offer,
            reach: reach_list,
            refused,
            request: None,
            certificate: None,
        });
        if refused.is_some() {
            return Ok(SshAccessChange::forget(
                self.state_event().into_iter().collect(),
            ));
        }
        let mut events: Vec<_> = self.state_event().into_iter().collect();
        events.extend(self.open_request(reach, subject, now, request));
        Ok(SshAccessChange::keep(events))
    }
    /// Accepts a device's certificate for the open request after checking that it
    /// certifies this key for exactly the requested servers within the request's time.
    pub fn certify(
        &mut self,
        offer: &SshCertificateOffer,
        facts: &CertificateFacts,
        subject: &[u8; 32],
        now: Timestamp,
    ) -> Result<SshAccessChange, SshAccessError> {
        let binding = self
            .parts
            .binding
            .as_mut()
            .filter(|binding| binding.refused.is_none())
            .ok_or(SshAccessError::NoBinding)?;
        if binding.offer.grant() != offer.grant() {
            return Err(SshAccessError::OtherGrant);
        }
        let open = binding
            .request
            .clone()
            .filter(|open| open.id == *offer.request())
            .ok_or(SshAccessError::UnknownRequest)?;
        if facts.subject_key != *subject {
            return Err(SshAccessError::SubjectKey);
        }
        let mut expected: Vec<_> = open
            .servers
            .iter()
            .filter_map(|server| binding.offer.target(*server))
            .map(|target| target.principal.as_str())
            .collect();
        let mut named: Vec<_> = facts.principals.iter().map(String::as_str).collect();
        expected.sort_unstable();
        named.sort_unstable();
        named.dedup();
        if expected != named {
            return Err(SshAccessError::Principals);
        }
        let life = facts.valid_before.elapsed_since(facts.valid_after);
        if facts.valid_before <= now
            || facts.valid_before > open.not_after
            || facts.valid_before > binding.offer.expires()
            || life.is_none_or(|life| life == 0 || life > MAX_CERTIFICATE_VALIDITY_MS + BACKDATE_MS)
        {
            return Err(SshAccessError::Validity);
        }
        binding.certificate = Some(IssuedCertificate {
            request: open.id,
            servers: open.servers.clone(),
            blob: offer.certificate().clone(),
            valid_after: facts.valid_after,
            valid_before: facts.valid_before,
        });
        binding.request = None;
        Ok(SshAccessChange::keep(
            self.state_event().into_iter().collect(),
        ))
    }
    /// A device revoked the chat's binding: access ends and the key is forgotten.
    pub fn revoke(&mut self, grant: GrantReference) -> Result<SshAccessChange, SshAccessError> {
        let current = self
            .parts
            .binding
            .as_ref()
            .ok_or(SshAccessError::NoBinding)?
            .offer
            .grant();
        if current.id() != grant.id() || grant.epoch() < current.epoch() {
            return Err(SshAccessError::OtherGrant);
        }
        let mut report = self.report().ok_or(SshAccessError::NoBinding)?;
        report.access = SshAccessPhase::Revoked;
        self.parts.binding = None;
        Ok(SshAccessChange::forget(vec![SshAccessEvent::State(report)]))
    }
    /// The chat was deleted: everything ends without an event.
    pub fn forget(&mut self) -> SshAccessChange {
        self.parts.binding = None;
        SshAccessChange::forget(Vec::new())
    }
    /// Ends an expired binding, lapses an expired certificate and asks for the next
    /// certificate at 80 % of the current one's life or when a request went unanswered.
    pub fn tick(
        &mut self,
        reach: &SshReach,
        subject: &[u8; 32],
        now: Timestamp,
        request: [u8; 16],
    ) -> SshAccessChange {
        let Some(expires) = self
            .parts
            .binding
            .as_ref()
            .filter(|binding| binding.refused.is_none())
            .map(|binding| binding.offer.expires())
        else {
            return SshAccessChange::keep(Vec::new());
        };
        if now >= expires {
            let events = self
                .report()
                .map(|mut report| {
                    report.access = SshAccessPhase::Expired;
                    SshAccessEvent::State(report)
                })
                .into_iter()
                .collect();
            self.parts.binding = None;
            return SshAccessChange::forget(events);
        }
        let mut events = Vec::new();
        if let Some(binding) = self.parts.binding.as_mut()
            && binding
                .certificate
                .as_ref()
                .is_some_and(|certificate| now >= certificate.valid_before)
        {
            binding.certificate = None;
            events.extend(self.state_event());
        }
        let due = self.parts.binding.as_ref().is_some_and(|binding| {
            match (&binding.certificate, &binding.request) {
                (_, Some(open)) => now >= open.not_after,
                (Some(certificate), None) => now >= renew_at(certificate),
                (None, None) => true,
            }
        });
        if due {
            events.extend(self.open_request(reach, subject, now, request));
        }
        SshAccessChange::keep(events)
    }
    /// The next time `tick` has something to do.
    pub fn next_due(&self) -> Option<Timestamp> {
        let binding = self.parts.binding.as_ref()?;
        if binding.refused.is_some() {
            return None;
        }
        let mut due = binding.offer.expires();
        if let Some(open) = &binding.request {
            due = due.min(open.not_after);
        }
        if let Some(certificate) = &binding.certificate {
            due = due.min(certificate.valid_before);
            if binding.request.is_none() {
                due = due.min(renew_at(certificate));
            }
        }
        Some(due)
    }
    /// The certificate for `endpoint` when one is active and names its bound server.
    pub fn credential(
        &self,
        endpoint: &SshEndpoint,
        host_key: &Fingerprint,
        now: Timestamp,
    ) -> Option<SshCredential<'_>> {
        let binding = self.parts.binding.as_ref()?;
        let certificate = binding.certificate.as_ref()?;
        if now >= certificate.valid_before || now >= binding.offer.expires() {
            return None;
        }
        let target = binding.offer.targets().iter().find(|target| {
            &target.endpoint == endpoint
                && &target.host_key == host_key
                && certificate.servers.contains(&target.server)
        })?;
        Some(SshCredential {
            server: target.server,
            principal: &target.principal,
            certificate: &certificate.blob,
            valid_before: certificate.valid_before,
        })
    }
    /// Where the current binding stands, as its state event says.
    pub fn report(&self) -> Option<SshAccessReport> {
        let binding = self.parts.binding.as_ref()?;
        let access = match (&binding.refused, &binding.certificate) {
            (Some(refusal), _) => SshAccessPhase::Refused(*refusal),
            (None, Some(certificate)) => SshAccessPhase::Active {
                request: certificate.request,
                valid_before: certificate.valid_before,
            },
            (None, None) => SshAccessPhase::Pending,
        };
        Some(SshAccessReport {
            grant: binding.offer.grant(),
            access,
            servers: binding.reach.clone(),
        })
    }
    /// The bound targets with their reach, for the chat's summary.
    pub fn binding(&self) -> Option<&SshBindingOffer> {
        self.parts.binding.as_ref().map(|binding| &binding.offer)
    }
    fn state_event(&self) -> Option<SshAccessEvent> {
        self.report().map(SshAccessEvent::State)
    }
    fn note_epoch(&mut self, grant: GrantReference) -> Result<(), SshAccessError> {
        let epochs = &mut self.parts.epochs;
        match epochs.iter_mut().find(|(id, _)| *id == grant.id()) {
            Some((_, epoch)) if *epoch >= grant.epoch() => return Err(SshAccessError::Stale),
            Some((_, epoch)) => *epoch = grant.epoch(),
            None => {
                if epochs.len() >= MAX_KNOWN_GRANTS {
                    epochs.remove(0);
                }
                epochs.push((grant.id(), grant.epoch()));
            }
        }
        Ok(())
    }
    fn open_request(
        &mut self,
        reach: &SshReach,
        subject: &[u8; 32],
        now: Timestamp,
        id: [u8; 16],
    ) -> Option<SshAccessEvent> {
        let binding = self.parts.binding.as_mut()?;
        let not_after = Timestamp::from_millis(
            now.as_millis()
                .saturating_add(DEFAULT_CERTIFICATE_VALIDITY_MS),
        )
        .min(binding.offer.expires());
        let servers: Vec<_> = binding
            .reach
            .iter()
            .filter(|(_, reach)| *reach == SshServerReach::Admitted)
            .map(|(server, _)| *server)
            .collect();
        if not_after <= now || servers.is_empty() || reach.egress().is_empty() {
            return None;
        }
        binding.request = Some(OpenRequest {
            id,
            servers: servers.clone(),
            at: now,
            not_after,
        });
        Some(SshAccessEvent::Requested(Box::new(SshCertificateRequest {
            request: id,
            binding: binding.offer.signed().clone(),
            subject_key: *subject,
            source_addresses: reach.egress().to_vec(),
            servers,
            not_after,
        })))
    }
}

/// The time at which a certificate's successor is requested.
fn renew_at(certificate: &IssuedCertificate) -> Timestamp {
    let life = certificate
        .valid_before
        .elapsed_since(certificate.valid_after)
        .unwrap_or(0);
    Timestamp::from_millis(
        certificate
            .valid_after
            .as_millis()
            .saturating_add(life / 100 * RENEW_AT_PERCENT),
    )
}
