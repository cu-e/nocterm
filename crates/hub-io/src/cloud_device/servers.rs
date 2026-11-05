//! A chat's SSH binding as the neutral servers of the desktop (ADR 0139): the core's verified
//! `ChatSsh` mapped to a [`ServerBinding`], and its SSH refusals to [`ServerRefusal`]s.
use futures::future::BoxFuture;
use nocterm_cloud_conversation_model::{
    BoundServer, CloudConversationError, ServerAccess, ServerBinding, ServerChoice, ServerKey,
    ServerRefusal,
};
use nocterm_cloud_device::SshRefusal;
use nocterm_cloud_device::ssh::SshServerChoice;
use nocterm_cloud_device::transcript::ChatSsh;
use nocterm_hub_model::cloud::ssh::{SshAccess, SshAccessUpdate, SshServerId, SshServerReach};

/// The servers of the account's CA registry a chat of its hub can be bound to.
pub(crate) trait ServerDirectory: Send + Sync {
    fn choices(&self) -> BoxFuture<'static, Result<Vec<ServerChoice>, CloudConversationError>>;
}

/// The core's choices for `keys`: each must be listed in `known` with a port.
pub(crate) fn chosen(
    keys: &[ServerKey],
    known: &[ServerChoice],
) -> Result<Vec<SshServerChoice>, CloudConversationError> {
    keys.iter()
        .map(|key| {
            let choice = known
                .iter()
                .find(|choice| &choice.server == key)
                .ok_or(CloudConversationError::Servers(ServerRefusal::Untrusted))?;
            let port = choice
                .port
                .ok_or(CloudConversationError::Servers(ServerRefusal::UnknownPort))?;
            Ok(SshServerChoice {
                server: server_id(key),
                port,
            })
        })
        .collect()
}

pub(crate) fn key(server: &SshServerId) -> ServerKey {
    ServerKey::from_bytes(*server.as_bytes())
}
pub(crate) fn server_id(key: &ServerKey) -> SshServerId {
    SshServerId::from_bytes(*key.as_bytes())
}

/// The chat's current binding at `now_ms`: each bound server with the state the runner last
/// reported, and whether a prompt may name it now. Empty until the runner echoed a binding.
pub(crate) fn binding(ssh: &ChatSsh, now_ms: i64) -> ServerBinding {
    let Some(current) = ssh.current() else {
        return ServerBinding::default();
    };
    let Some(signed) = current.binding() else {
        return ServerBinding::default();
    };
    let binding = signed.binding();
    let selectable = ssh.session_targets(now_ms);
    let withdrawn = current.withdrawn();
    let servers = binding
        .targets
        .iter()
        .map(|target| BoundServer {
            server: key(&target.server),
            host: target.host.as_str().to_owned(),
            port: target.port,
            user: target.user.as_str().to_owned(),
            access: if withdrawn {
                ServerAccess::Revoked
            } else {
                access(current.update(), &target.server, now_ms)
            },
            selectable: !withdrawn && selectable.iter().any(|known| known.server == target.server),
        })
        .collect();
    ServerBinding {
        servers,
        expires_at_ms: Some(binding.window.expires_at_ms),
    }
}

/// Where `server` stands by the runner's latest report.
fn access(update: Option<&SshAccessUpdate>, server: &SshServerId, now_ms: i64) -> ServerAccess {
    let Some(update) = update else {
        return ServerAccess::Pending;
    };
    let reach = update
        .servers
        .iter()
        .find(|status| &status.server == server)
        .map(|status| status.reach);
    let admitted = reach == Some(SshServerReach::Admitted);
    match update.access {
        SshAccess::Refused(_) => ServerAccess::Refused,
        SshAccess::Revoked => ServerAccess::Revoked,
        SshAccess::Expired => ServerAccess::Expired,
        _ if !admitted => ServerAccess::Unreachable,
        SshAccess::Active {
            valid_before_ms, ..
        } if valid_before_ms > now_ms => ServerAccess::Active {
            until_ms: valid_before_ms,
        },
        SshAccess::Pending | SshAccess::Active { .. } => ServerAccess::Pending,
    }
}

/// The refusal a person can act on, or `None` for one that only a later round changes.
pub(crate) fn refusal(refusal: SshRefusal) -> Option<ServerRefusal> {
    Some(match refusal {
        SshRefusal::NoCa => ServerRefusal::NoCa,
        SshRefusal::OtherCa
        | SshRefusal::UntrustedServer
        | SshRefusal::HostKeyUnknown
        | SshRefusal::NoTrustedServer => ServerRefusal::Untrusted,
        SshRefusal::Servers => ServerRefusal::Servers,
        SshRefusal::Window => ServerRefusal::Window,
        SshRefusal::NotBound => ServerRefusal::NotBound,
        SshRefusal::NotSelectable => ServerRefusal::NotSelectable,
        SshRefusal::UnknownRequest
        | SshRefusal::Superseded
        | SshRefusal::Revoked
        | SshRefusal::OutsideBinding
        | SshRefusal::SubjectKeyChanged => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_hub_model::cloud::payload::GrantReference;
    use nocterm_hub_model::cloud::ssh::{SshBindingRefusal, SshServerStatus};

    fn update(access: SshAccess, admitted: &[&str]) -> SshAccessUpdate {
        SshAccessUpdate {
            grant: GrantReference {
                id: [1; 16],
                epoch: 1,
            },
            access,
            servers: ["db", "web"]
                .iter()
                .map(|profile| SshServerStatus {
                    server: SshServerId::of_profile(profile),
                    reach: if admitted.contains(profile) {
                        SshServerReach::Admitted
                    } else {
                        SshServerReach::Unreachable
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn a_bound_server_shows_pending_active_until_or_why_it_cannot_be_used() {
        let db = SshServerId::of_profile("db");
        let web = SshServerId::of_profile("web");
        let active = SshAccess::Active {
            request: [2; 16],
            valid_before_ms: 500,
        };
        assert_eq!(access(None, &db, 100), ServerAccess::Pending);
        let report = update(active, &["db"]);
        assert_eq!(
            access(Some(&report), &db, 100),
            ServerAccess::Active { until_ms: 500 }
        );
        assert_eq!(access(Some(&report), &db, 500), ServerAccess::Pending);
        assert_eq!(access(Some(&report), &web, 100), ServerAccess::Unreachable);
        for (state, expected) in [
            (
                SshAccess::Refused(SshBindingRefusal::NoSourceAddress),
                ServerAccess::Refused,
            ),
            (SshAccess::Revoked, ServerAccess::Revoked),
            (SshAccess::Expired, ServerAccess::Expired),
        ] {
            assert_eq!(access(Some(&update(state, &["db"])), &db, 100), expected);
        }
    }

    #[test]
    fn an_unbound_chat_has_no_servers_and_refusals_keep_what_the_person_can_change() {
        assert_eq!(binding(&ChatSsh::default(), 0), ServerBinding::default());
        assert_eq!(
            refusal(SshRefusal::NotSelectable),
            Some(ServerRefusal::NotSelectable)
        );
        assert_eq!(
            refusal(SshRefusal::UntrustedServer),
            Some(ServerRefusal::Untrusted)
        );
        assert_eq!(refusal(SshRefusal::OutsideBinding), None);
        let key = key(&SshServerId::of_profile("db"));
        assert_eq!(server_id(&key), SshServerId::of_profile("db"));
    }
}
