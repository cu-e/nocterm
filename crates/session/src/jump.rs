//! Jump-host route values: the hops a transport traverses before the destination.
//!
//! A route is resolved before a session opens (a saved profile reference becomes a
//! target, an authentication method and a credential), so a transport receives only
//! these plain values.

use crate::{Auth, CredentialId, Target};

/// The most jump hosts one route may traverse.
pub const MAX_JUMP_HOSTS: usize = 8;

/// One hop of a route: where to connect and how to authenticate there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpHost {
    pub target: Target,
    pub auth: Auth,
}

/// A hop together with the saved credential that answers its secret prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedJump {
    pub host: JumpHost,
    pub credential: Option<CredentialId>,
}

/// A machine as the user reached it: the destination and the jump hosts in
/// front of it. Equal targets behind different routes are different machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAddress {
    pub target: Target,
    pub via: Vec<Target>,
}

impl HostAddress {
    pub fn new(target: Target, via: Vec<Target>) -> Self {
        Self { target, via }
    }

    /// A destination reached directly.
    pub fn direct(target: Target) -> Self {
        Self::new(target, Vec::new())
    }
}

impl std::fmt::Display for HostAddress {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.target.fmt(output)?;
        match self.via.first() {
            Some(first) => write!(output, " via {first}"),
            None => Ok(()),
        }
    }
}

/// The hop a prompt belongs to. A prompt without one is for the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHop {
    /// Zero-based position in the route.
    pub index: usize,
    pub target: Target,
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;
    use crate::SessionError;

    fn bastion() -> Target {
        Target::new("ops", "bastion", 22)
    }

    #[test]
    fn hop_values_compare_by_target_auth_and_credential() {
        let host = JumpHost {
            target: bastion(),
            auth: Auth::Auto,
        };
        let credential = Some(CredentialId::from_bytes([7; 16]));
        let hop = ResolvedJump {
            host: host.clone(),
            credential,
        };
        assert_eq!(
            hop,
            ResolvedJump {
                host: host.clone(),
                credential
            }
        );
        assert_ne!(
            hop,
            ResolvedJump {
                host,
                credential: None
            }
        );
    }

    #[test]
    fn a_routed_address_names_the_first_jump_host_so_equal_targets_stay_apart() {
        let target = Target::new("root", "192.168.122.10", 22);
        let other = Target::new("ops", "other", 22);

        assert_eq!(
            HostAddress::direct(target.clone()).to_string(),
            "root@192.168.122.10"
        );
        assert_eq!(
            HostAddress::new(target.clone(), vec![bastion(), other]).to_string(),
            "root@192.168.122.10 via ops@bastion"
        );
    }

    #[test]
    fn a_jump_host_failure_names_the_hop_and_keeps_its_cause() {
        let error = SessionError::JumpHost {
            hop: 0,
            host: bastion().to_string(),
            source: Box::new(SessionError::Unreachable {
                host: "db".into(),
                reason: "forwarding refused".into(),
            }),
        };
        assert_eq!(
            error.to_string(),
            "jump host 1 (ops@bastion): could not reach db: forwarding refused"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("could not reach db: forwarding refused")
        );
    }

    #[test]
    fn an_oversized_route_reports_count_and_limit() {
        let error = SessionError::TooManyJumpHosts {
            count: 9,
            limit: MAX_JUMP_HOSTS,
        };
        assert_eq!(error.to_string(), "9 jump hosts exceed the limit of 8");
    }
}
