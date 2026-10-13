//! Recent destinations and supplied observation timestamps.
use crate::profiles::ProfileId;
use nocterm_session::{Auth, CredentialId, ShellLaunch, Target};
use serde::{Deserialize, Serialize};
pub const MAX_RECENTS: usize = 8;
fn is_auto(auth: &Auth) -> bool {
    *auth == Auth::Auto
}
/// A connection that was opened, saved or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    #[serde(
        default,
        skip_serializing_if = "nocterm_session::SessionOptions::is_default"
    )]
    pub options: nocterm_session::SessionOptions,
    #[serde(flatten)]
    pub target: Target,
    #[serde(default, skip_serializing_if = "is_auto")]
    pub auth: Auth,
    /// The saved connection it was opened from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileId>,
    /// Seconds since the Unix epoch.
    pub last_used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<ShellLaunch>,
}

impl Recent {
    /// Whether the two would open the same thing.
    fn same_as(&self, other: &Self) -> bool {
        match (self.profile, other.profile) {
            (Some(left), Some(right)) => left == right,
            (None, None) => {
                self.target == other.target
                    && self.auth == other.auth
                    && self.credential == other.credential
                    && self.launch == other.launch
                    && self.options == other.options
            }
            _ => false,
        }
    }
}

/// The contents of `recents.toml`, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recents {
    #[serde(default, rename = "recent")]
    list: Vec<Recent>,
}

impl Recents {
    pub fn iter(&self) -> impl Iterator<Item = &Recent> {
        self.list.iter()
    }

    pub fn associate_credential(&mut self, target: &Target, auth: &Auth, id: CredentialId) {
        for recent in self
            .list
            .iter_mut()
            .filter(|recent| &recent.target == target && &recent.auth == auth)
        {
            recent.credential = Some(id);
        }
    }

    /// Puts `recent` first, dropping an older entry for the same thing and
    /// whatever falls off the end.
    pub fn record(&mut self, recent: Recent) {
        self.list.retain(|existing| !existing.same_as(&recent));
        self.list.insert(0, recent);
        self.list.truncate(MAX_RECENTS);
    }

    /// Forgets that entries came from a profile that is gone; they stay as
    /// plain destinations.
    pub fn unlink(&mut self, id: ProfileId) {
        for recent in &mut self.list {
            if recent.profile == Some(id) {
                recent.profile = None;
            }
        }
        // Unlinking can leave two entries for the same destination.
        let mut seen: Vec<Recent> = Vec::new();
        self.list.retain(|recent| {
            let duplicate = seen.iter().any(|earlier| earlier.same_as(recent));
            seen.push(recent.clone());
            !duplicate
        });
    }
}
