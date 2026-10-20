//! Saved jump-host references and their expansion into a route.
//!
//! A profile lists its jump hosts in order. Each is a reference to another saved
//! connection or a typed `user@host[:port]`. Opening the profile expands the list
//! into plain values the session contract understands.
use nocterm_session::{Auth, JumpHost, MAX_JUMP_HOSTS, ResolvedJump, Target};
use serde::{Deserialize, Serialize};

use crate::profiles::{Profile, ProfileId, Profiles};

/// One saved jump host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JumpHostRecord", into = "JumpHostRecord")]
pub enum JumpHostEntry {
    /// A saved connection: its target, authentication and credential are used.
    Profile(ProfileId),
    /// A typed address, reached with automatic authentication and no saved credential.
    Address(Target),
}

/// The TOML shape of an entry: `profile`, or `host` with `user` and an optional `port`.
#[derive(Serialize, Deserialize)]
struct JumpHostRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

/// A saved jump-host entry that is not well formed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JumpHostRecordError {
    #[error(
        "a jump host names either a saved connection (`profile`) or an address (`host`), not both"
    )]
    Both,
    #[error("a jump host needs a saved connection (`profile`) or an address (`host`)")]
    Neither,
    #[error("the jump host `{host}` needs a `user`")]
    MissingUser { host: String },
}

impl TryFrom<JumpHostRecord> for JumpHostEntry {
    type Error = JumpHostRecordError;

    fn try_from(record: JumpHostRecord) -> Result<Self, Self::Error> {
        match (record.profile, record.host) {
            (Some(_), host) if host.is_some() || record.user.is_some() || record.port.is_some() => {
                Err(JumpHostRecordError::Both)
            }
            (Some(id), _) => Ok(Self::Profile(id)),
            (None, None) => Err(JumpHostRecordError::Neither),
            (None, Some(host)) => {
                let user = record
                    .user
                    .ok_or_else(|| JumpHostRecordError::MissingUser { host: host.clone() })?;
                Ok(Self::Address(Target::new(
                    user,
                    host,
                    record.port.unwrap_or(nocterm_session::DEFAULT_PORT),
                )))
            }
        }
    }
}

impl From<JumpHostEntry> for JumpHostRecord {
    fn from(entry: JumpHostEntry) -> Self {
        match entry {
            JumpHostEntry::Profile(id) => Self {
                profile: Some(id),
                host: None,
                port: None,
                user: None,
            },
            JumpHostEntry::Address(target) => Self {
                profile: None,
                host: Some(target.host),
                port: (target.port != nocterm_session::DEFAULT_PORT).then_some(target.port),
                user: Some(target.user),
            },
        }
    }
}

/// Why a profile's jump hosts cannot be expanded into a route.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JumpRouteError {
    #[error("{referrer} uses a jump host that is no longer a saved connection")]
    MissingProfile {
        referrer: String,
        missing: ProfileId,
    },
    #[error("{profile} would jump through itself")]
    Cycle { profile: String },
    #[error("a route may pass through at most {limit} jump hosts")]
    TooLong { limit: usize },
}

/// Expands the jump hosts of `profile` into the hops to traverse, nearest first.
///
/// A referenced connection's own jump hosts come before it, as with OpenSSH. A
/// hop takes only the target, authentication and credential of its connection.
/// `profile` is used as given, so an unsaved edit of a connection takes part in
/// cycle detection in place of the stored one.
pub fn resolve_jump_route(
    profiles: &Profiles,
    profile: &Profile,
) -> Result<Vec<ResolvedJump>, JumpRouteError> {
    let mut route = Vec::new();
    expand(profiles, profile, &mut vec![profile.id], &mut route)?;
    Ok(route)
}

fn expand(
    profiles: &Profiles,
    profile: &Profile,
    stack: &mut Vec<ProfileId>,
    route: &mut Vec<ResolvedJump>,
) -> Result<(), JumpRouteError> {
    for entry in &profile.jump_hosts {
        match entry {
            JumpHostEntry::Address(target) => push(
                route,
                ResolvedJump {
                    host: JumpHost {
                        target: target.clone(),
                        auth: Auth::Auto,
                    },
                    credential: None,
                },
            )?,
            JumpHostEntry::Profile(id) => {
                let referenced =
                    profiles
                        .get(*id)
                        .ok_or_else(|| JumpRouteError::MissingProfile {
                            referrer: profile.name.clone(),
                            missing: *id,
                        })?;
                if stack.contains(id) {
                    return Err(JumpRouteError::Cycle {
                        profile: referenced.name.clone(),
                    });
                }
                stack.push(*id);
                expand(profiles, referenced, stack, route)?;
                stack.pop();
                push(
                    route,
                    ResolvedJump {
                        host: JumpHost {
                            target: referenced.target.clone(),
                            auth: referenced.auth.clone(),
                        },
                        credential: referenced.credential,
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn push(route: &mut Vec<ResolvedJump>, hop: ResolvedJump) -> Result<(), JumpRouteError> {
    if route.len() >= MAX_JUMP_HOSTS {
        return Err(JumpRouteError::TooLong {
            limit: MAX_JUMP_HOSTS,
        });
    }
    route.push(hop);
    Ok(())
}

#[cfg(test)]
mod tests;
