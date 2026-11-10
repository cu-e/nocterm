//! The contract's endpoint for a hub and the trust decided for it.

use nocterm_hub_client::{HubEndpoint, Trust};
use nocterm_hubs_model::{HubEndpointSpec, HubError, TrustPolicy};

use crate::classify::remote;

pub(crate) fn endpoint(spec: &HubEndpointSpec) -> Result<HubEndpoint, HubError> {
    let trust = match spec.trust {
        TrustPolicy::Pinned(fingerprint) => Trust::Pinned(*fingerprint.as_bytes()),
        TrustPolicy::PublicCa => Trust::PublicCa,
    };
    HubEndpoint::with_trust(&spec.address.to_string(), trust).map_err(remote)
}

#[cfg(test)]
mod tests {
    use nocterm_hubs_model::{Fingerprint, HubAddress};

    use super::*;

    fn spec(address: &str, trust: TrustPolicy) -> HubEndpointSpec {
        HubEndpointSpec {
            address: HubAddress::parse(address).unwrap(),
            trust,
        }
    }

    #[test]
    fn the_decided_trust_becomes_the_contracts_trust() {
        let pinned = Fingerprint::from_bytes([9; 32]);
        let endpoint = endpoint(&spec("hub.example:9000", TrustPolicy::Pinned(pinned))).unwrap();
        assert_eq!(endpoint.host(), "hub.example");
        assert_eq!(endpoint.port(), 9000);
        assert_eq!(endpoint.trust(), &Trust::Pinned([9; 32]));

        let public = super::endpoint(&spec("hub.example", TrustPolicy::PublicCa)).unwrap();
        assert_eq!(public.port(), 7443);
        assert_eq!(public.trust(), &Trust::PublicCa);
    }

    #[test]
    fn an_ipv6_address_keeps_its_brackets_out_of_the_host() {
        let endpoint = endpoint(&spec("[::1]:7443", TrustPolicy::PublicCa)).unwrap();
        assert_eq!(endpoint.host(), "::1");
        assert_eq!(endpoint.port(), 7443);
    }
}
