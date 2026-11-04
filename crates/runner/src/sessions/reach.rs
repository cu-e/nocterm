//! What this runner states for chat SSH bindings (ADR 0137): the servers it may reach
//! and the egress addresses its connections come from.
//!
//! Reach is selected, never constructed: a bound server is admitted only when it is one
//! of the administrator's configured SSH targets (same host, port and user, and the
//! configured host key equals the binding's) or one of a managed boot's targets (same
//! host, port and user; the binding's host key is then pinned). Egress comes from
//! `[ssh] egress` of an own runner or `ssh_egress` of boot v3; without it every binding
//! is refused with `NoSourceAddress`.
use super::config::SessionTargetConfiguration;
use crate::RunnerError;
use nocterm_cloud_model::ssh_access::{SshEndpoint, SshReach};
use serde::Deserialize;

/// At most this many egress addresses, as a certificate request carries them.
const MAX_EGRESS: usize = 8;

/// `[ssh]` of runner.toml.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SshConfiguration {
    egress: Vec<String>,
}
impl SshConfiguration {
    pub(crate) fn egress(self) -> Result<Vec<String>, RunnerError> {
        if !valid_egress(&self.egress) {
            return Err(RunnerError::Value("ssh.egress"));
        }
        Ok(self.egress)
    }
}

/// Whether `egress` can be stated in a certificate request: 1 to 8 IP addresses or CIDRs.
pub(crate) fn valid_egress(egress: &[String]) -> bool {
    (1..=MAX_EGRESS).contains(&egress.len())
        && egress.iter().all(|entry| {
            let (address, prefix) = entry.split_once('/').unwrap_or((entry, ""));
            let prefix_ok = |maximum: u8| {
                prefix.is_empty() || prefix.parse::<u8>().is_ok_and(|bits| bits <= maximum)
            };
            entry.len() <= 64
                && match address.parse::<std::net::IpAddr>() {
                    Ok(std::net::IpAddr::V4(_)) => prefix_ok(32),
                    Ok(std::net::IpAddr::V6(_)) => prefix_ok(128),
                    Err(_) => false,
                }
        })
}

/// A managed boot's SSH targets and egress, which replace runner.toml's for that runner.
pub(crate) struct BootSsh {
    targets: Vec<SshEndpoint>,
    egress: Vec<String>,
}
impl BootSsh {
    pub(crate) fn new<'a>(
        targets: impl Iterator<Item = (&'a str, u16, &'a str)>,
        egress: &[String],
    ) -> Self {
        Self {
            targets: targets
                .map(|(host, port, user)| SshEndpoint {
                    host: host.to_owned(),
                    port,
                    user: user.to_owned(),
                })
                .collect(),
            egress: egress.to_vec(),
        }
    }
}

/// The reach of a runner with `targets` and either its own `egress` or a managed `boot`.
pub(crate) fn reach(
    targets: &SessionTargetConfiguration,
    egress: Vec<String>,
    boot: Option<BootSsh>,
) -> Result<SshReach, RunnerError> {
    let mut endpoints: Vec<_> = targets
        .ssh_endpoints()?
        .into_iter()
        .map(|(endpoint, host_key)| (endpoint, Some(host_key)))
        .collect();
    let egress = match boot {
        Some(boot) => {
            endpoints.extend(boot.targets.into_iter().map(|endpoint| (endpoint, None)));
            boot.egress
        }
        None => egress,
    };
    SshReach::new(endpoints, egress).map_err(|_| RunnerError::Value("ssh.reach"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::{
        identity::Fingerprint,
        ssh_access::{BoundTarget, SshServerKey, SshServerReach},
    };

    fn targets() -> SessionTargetConfiguration {
        let host = nocterm_ssh_keys::KeyMaterial::ed25519_from_seed(&[1; 32], "h")
            .unwrap()
            .public_key();
        let fingerprint = host
            .fingerprint_bytes()
            .unwrap()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        toml::from_str(&format!(
            "default = \"db\"\n[[targets]]\nname = \"db\"\nhost_fingerprint = \"{fingerprint}\"\nroute_fingerprint = \"{r}\"\nprincipal = \"deploy\"\nworkspace_fingerprint = \"{r}\"\nauthentication_key = \"{r}\"\ncredential_revision = \"{r}\"\n[targets.connector]\nkind = \"ssh\"\nhost = \"db.example\"\nport = 2222\nhost_public_key = \"{}\"\nterminal = \"xterm-256color\"\ncolumns = 80\nrows = 24\ntimeout_ms = 10000\n[targets.connector.launch]\n",
            host.as_str(),
            r = "02".repeat(32),
        ))
        .unwrap()
    }
    fn bound(host: &str, port: u16, key: Fingerprint) -> BoundTarget {
        BoundTarget {
            server: SshServerKey::from_bytes([1; 16]),
            endpoint: SshEndpoint {
                host: host.into(),
                port,
                user: "deploy".into(),
            },
            host_key: key,
            principal: "nocterm-agent-x".into(),
        }
    }

    #[test]
    fn configured_targets_need_their_host_key_and_boot_targets_pin_the_bindings() {
        let configured = targets();
        let (_, key) = configured.ssh_endpoints().unwrap().remove(0);
        let boot = BootSsh::new(
            [("boot.example", 22, "deploy")].into_iter(),
            &["203.0.113.7".into()],
        );
        let reach = reach(&configured, vec!["10.9.9.9".into()], Some(boot)).unwrap();
        assert_eq!(
            reach.admits(&bound("db.example", 2222, key)),
            SshServerReach::Admitted
        );
        let other = Fingerprint::from_bytes([9; 32]);
        assert_eq!(
            reach.admits(&bound("db.example", 2222, other)),
            SshServerReach::Unreachable
        );
        assert_eq!(
            reach.admits(&bound("db.example", 22, key)),
            SshServerReach::Unreachable
        );
        assert_eq!(
            reach.admits(&bound("boot.example", 22, other)),
            SshServerReach::Admitted
        );
        assert_eq!(reach.egress(), ["203.0.113.7"]);
        let own = super::reach(&configured, vec!["10.9.9.9".into()], None).unwrap();
        assert_eq!(own.egress(), ["10.9.9.9"]);
        assert_eq!(
            own.admits(&bound("boot.example", 22, other)),
            SshServerReach::Unreachable
        );
    }

    #[test]
    fn egress_takes_addresses_and_cidrs_and_refuses_names_and_bad_prefixes() {
        assert!(valid_egress(&["10.0.0.1".into(), "2001:db8::/64".into()]));
        for bad in ["host.example", "10.0.0.0/33", "::1/129", "10.0.0.1/x", ""] {
            assert!(!valid_egress(&[bad.into()]), "{bad}");
        }
        assert!(!valid_egress(&[]));
        assert!(!valid_egress(&vec!["10.0.0.1".into(); 9]));
    }
}
