//! What this runner states for chat SSH bindings (ADR 0145): the servers it may reach
//! and the egress addresses its connections come from.
//!
//! Reach is selected, never constructed: a bound server is admitted only when it is one
//! of the administrator's configured SSH targets or one of a managed boot v3's targets
//! with a host key, with the same host, port and user and a host key equal to the
//! binding's (`BootSshTarget::admits`, ADR 0147). A boot target without a host key is
//! listed but never admitted. Egress comes from `[ssh] egress` of an own runner or
//! `ssh_egress` of boot v3; without it every binding is refused with `NoSourceAddress`.
use super::config::SessionTargetConfiguration;
use crate::RunnerError;
use nocterm_cloud_model::{
    identity::Fingerprint,
    ssh_access::{SshEndpoint, SshReach},
};
use nocterm_hub_io::runner_control::boot_ssh::{BootSshTarget, is_source_address};
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

/// Whether `egress` can be stated in a certificate request: 1 to 8 IP addresses or CIDRs
/// (the contract's source address rule).
pub(crate) fn valid_egress(egress: &[String]) -> bool {
    (1..=MAX_EGRESS).contains(&egress.len()) && egress.iter().all(|entry| is_source_address(entry))
}

/// A managed boot's SSH targets and egress, which replace runner.toml's for that runner.
pub(crate) struct BootSsh {
    targets: Vec<BootSshTarget>,
    egress: Vec<String>,
}
impl BootSsh {
    pub(crate) fn new(targets: &[BootSshTarget], egress: &[String]) -> Self {
        Self {
            targets: targets.to_vec(),
            egress: egress.to_vec(),
        }
    }
    /// The boot targets a binding can select: those with a host key.
    pub(crate) fn keyed_targets(&self) -> impl Iterator<Item = (&BootSshTarget, Fingerprint)> {
        self.targets
            .iter()
            .filter_map(|target| Some((target, Fingerprint::from_bytes(target.host_key_sha256()?))))
    }
}

#[cfg(target_os = "linux")]
impl BootSsh {
    /// Session connectors for the keyed boot targets, from each target's host key and
    /// session settings (the login shell in an `xterm-256color` 80 × 24 terminal when
    /// absent).
    pub(crate) fn connectors(
        &self,
        ssh: &std::sync::Arc<nocterm_ssh::SshTransport>,
    ) -> Result<Vec<super::targets::BootConnector>, RunnerError> {
        self.keyed_targets()
            .map(|(target, _)| {
                let host = target
                    .host_public_key
                    .as_deref()
                    .and_then(|key| nocterm_ssh_keys::PublicKeyText::parse(key).ok())
                    .ok_or(RunnerError::Value("boot.ssh_targets.host_public_key"))?;
                let session = target.session_or_default();
                let request = super::config::ssh_request(
                    nocterm_session::Target {
                        host: target.host.clone(),
                        port: target.port,
                        user: target.user.clone(),
                    },
                    nocterm_session::ShellLaunch {
                        program: session.program,
                        args: session.args,
                        cwd: session.cwd,
                        env: Default::default(),
                        integration: true,
                    },
                    session.terminal,
                    super::config::size(session.columns, session.rows),
                    std::time::Duration::from_millis(session.connect_timeout_ms),
                );
                Ok(super::targets::BootConnector::new(
                    ssh.clone(),
                    request,
                    host,
                )?)
            })
            .collect()
    }
}

fn endpoint_of(target: &BootSshTarget) -> SshEndpoint {
    SshEndpoint {
        host: target.host.clone(),
        port: target.port,
        user: target.user.clone(),
    }
}

/// The reach of a runner with `targets` and either its own `egress` or a managed `boot`.
pub(crate) fn reach(
    targets: &SessionTargetConfiguration,
    egress: Vec<String>,
    boot: Option<&BootSsh>,
) -> Result<SshReach, RunnerError> {
    let mut endpoints = targets.ssh_endpoints()?;
    let egress = match boot {
        Some(boot) => {
            endpoints.extend(
                boot.keyed_targets()
                    .map(|(target, host_key)| (endpoint_of(target), host_key)),
            );
            boot.egress.clone()
        }
        None => egress,
    };
    SshReach::new(endpoints, egress).map_err(|_| RunnerError::Value("ssh.reach"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::ssh_access::{BoundTarget, SshServerKey, SshServerReach};

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
            access: None,
        }
    }

    fn boot_target(host: &str, key: Option<&nocterm_ssh_keys::PublicKeyText>) -> BootSshTarget {
        BootSshTarget {
            host: host.into(),
            user: "deploy".into(),
            port: 22,
            host_public_key: key.map(|key| key.as_str().to_owned()),
            session: None,
        }
    }

    #[test]
    fn configured_and_boot_targets_are_admitted_only_with_the_bindings_host_key() {
        let configured = targets();
        let (_, key) = configured.ssh_endpoints().unwrap().remove(0);
        let boot_key = nocterm_ssh_keys::KeyMaterial::ed25519_from_seed(&[2; 32], "b")
            .unwrap()
            .public_key();
        let boot_fingerprint = Fingerprint::from_bytes(boot_key.fingerprint_bytes().unwrap());
        let boot = BootSsh::new(
            &[
                boot_target("boot.example", Some(&boot_key)),
                boot_target("keyless.example", None),
            ],
            &["203.0.113.7".into()],
        );
        let reach = reach(&configured, vec!["10.9.9.9".into()], Some(&boot)).unwrap();
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
            reach.admits(&bound("boot.example", 22, boot_fingerprint)),
            SshServerReach::Admitted
        );
        assert_eq!(
            reach.admits(&bound("boot.example", 22, other)),
            SshServerReach::Unreachable
        );
        // Listed without a host key: never admitted, whatever key the binding pins.
        assert_eq!(
            reach.admits(&bound("keyless.example", 22, other)),
            SshServerReach::Unreachable
        );
        assert_eq!(reach.egress(), ["203.0.113.7"]);
        let own = super::reach(&configured, vec!["10.9.9.9".into()], None).unwrap();
        assert_eq!(own.egress(), ["10.9.9.9"]);
        assert_eq!(
            own.admits(&bound("boot.example", 22, boot_fingerprint)),
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
