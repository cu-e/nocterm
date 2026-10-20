//! Which saved connections a remembered credential may be attached to.
//!
//! A secret typed for one machine must never answer another. Equal
//! `user@host:port` behind different jump hosts are different machines, so a
//! session reached through jump hosts binds only to the profile it was opened
//! from, and a direct session binds only to connections without jump hosts.
use nocterm_session::{Auth, Target};

use crate::{
    profiles::{Profile, ProfileId, Profiles},
    recents::Recent,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Reach {
    Direct,
    Through(ProfileId),
}

/// The session a credential was remembered for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialScope {
    target: Target,
    auth: Auth,
    reach: Reach,
}

impl CredentialScope {
    /// A session reached without jump hosts.
    pub fn direct(target: Target, auth: Auth) -> Self {
        Self {
            target,
            auth,
            reach: Reach::Direct,
        }
    }

    /// A session reached through jump hosts, opened from `profile`.
    pub fn routed(profile: ProfileId, target: Target, auth: Auth) -> Self {
        Self {
            target,
            auth,
            reach: Reach::Through(profile),
        }
    }

    /// The scope of a session, or none when it is routed yet not opened from a
    /// saved connection: there is nothing it can safely be attached to.
    pub fn of_session(
        target: Target,
        auth: Auth,
        profile: Option<ProfileId>,
        routed: bool,
    ) -> Option<Self> {
        if !routed {
            return Some(Self::direct(target, auth));
        }
        profile.map(|profile| Self::routed(profile, target, auth))
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    pub(crate) fn admits_profile(&self, profile: &Profile) -> bool {
        profile.target == self.target
            && profile.auth == self.auth
            && match self.reach {
                Reach::Direct => profile.jump_hosts.is_empty(),
                Reach::Through(id) => profile.id == id,
            }
    }

    /// A recent stores no route of its own; it inherits that of the profile it
    /// came from. One whose profile is gone was a direct connection, because
    /// deleting a routed profile removes its recents.
    pub(crate) fn admits_recent(&self, recent: &Recent, profiles: &Profiles) -> bool {
        recent.target == self.target
            && recent.auth == self.auth
            && match self.reach {
                Reach::Direct => recent
                    .profile
                    .and_then(|id| profiles.get(id))
                    .is_none_or(|profile| profile.jump_hosts.is_empty()),
                Reach::Through(id) => recent.profile == Some(id),
            }
    }
}
