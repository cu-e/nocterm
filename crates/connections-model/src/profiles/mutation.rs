//! Checked profile intents, independent of UI and persistence scheduling.
use std::sync::Arc;

use nocterm_session::{Auth, CredentialId, Target};

use super::{Profile, ProfileId, Profiles};

const MAX_PROFILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProfileError {
    #[error(
        "This connection changed or was deleted in another view. Reopen the editor to load its current values; your draft was not saved."
    )]
    Conflict(ProfileId),
    #[error("Connection no longer exists")]
    MissingProfile(ProfileId),
    #[error("Group no longer exists")]
    MissingGroup(String),
    #[error("Group name cannot be empty")]
    EmptyGroup,
    #[error("Group changed; review it again.")]
    GroupChanged(String),
    #[error("Connection mutation exceeds the 64 KiB size limit.")]
    MutationSize,
    #[error("Connection fields exceed the 64 KiB size limit.")]
    ProfileSize,
    #[error("Could not encode connection fields: {0}")]
    Encoding(#[source] Arc<toml::ser::Error>),
}

#[derive(Clone)]
pub enum ProfileExpectation {
    Any,
    Absent,
    Snapshot(Box<Profile>),
}

#[derive(Clone)]
pub enum ProfileMutation {
    Save {
        profile: Box<Profile>,
        expected: ProfileExpectation,
    },
    Move {
        id: ProfileId,
        group: Option<String>,
        before: Option<ProfileId>,
    },
    Delete(ProfileId),
    RenameGroup {
        from: String,
        to: String,
    },
    Ungroup(String),
    DeleteGroup {
        group: String,
        expected: Vec<ProfileId>,
    },
    Credential {
        target: Target,
        auth: Auth,
        id: CredentialId,
    },
}

impl ProfileMutation {
    pub fn validate(&self) -> Result<(), ProfileError> {
        let metadata_bytes = match self {
            Self::Move { group, .. } => group.as_ref().map_or(0, String::len),
            Self::RenameGroup { from, to } => from.len().saturating_add(to.len()),
            Self::Ungroup(group) | Self::DeleteGroup { group, .. } => group.len(),
            Self::Credential { target, auth, .. } => target
                .host
                .len()
                .saturating_add(target.user.len())
                .saturating_add(
                    serde_json::to_string(auth).map_or(MAX_PROFILE_BYTES + 1, |text| text.len()),
                ),
            _ => 0,
        };
        if metadata_bytes > MAX_PROFILE_BYTES {
            return Err(ProfileError::MutationSize);
        }
        if let Self::Save { profile, .. } = self {
            let text = toml::to_string(profile.as_ref())
                .map_err(|error| ProfileError::Encoding(Arc::new(error)))?;
            if text.len() > MAX_PROFILE_BYTES {
                return Err(ProfileError::ProfileSize);
            }
        }
        Ok(())
    }

    pub fn apply(&self, profiles: &mut Profiles) -> Result<(), ProfileError> {
        match self {
            Self::Save { profile, expected } => {
                let current = profiles.get(profile.id);
                let matches = match expected {
                    ProfileExpectation::Any => true,
                    ProfileExpectation::Absent => current.is_none(),
                    ProfileExpectation::Snapshot(expected) => current == Some(expected.as_ref()),
                };
                if !matches {
                    return Err(ProfileError::Conflict(profile.id));
                }
                profiles.upsert(*profile.clone());
            }
            Self::Move { id, group, before } => {
                profiles.place(*id, group.clone(), *before)?;
            }
            Self::Delete(id) => {
                profiles.remove(*id);
            }
            Self::RenameGroup { from, to } => {
                require_group(profiles, from)?;
                let to = to.trim();
                if to.is_empty() {
                    return Err(ProfileError::EmptyGroup);
                }
                profiles.rename_group(from, to);
            }
            Self::Ungroup(group) => {
                require_group(profiles, group)?;
                profiles.ungroup(group);
            }
            Self::DeleteGroup { group, expected } => {
                require_group(profiles, group)?;
                let mut current: Vec<_> = profiles
                    .iter()
                    .filter(|profile| profile.group.as_deref() == Some(group))
                    .map(|profile| profile.id)
                    .collect();
                let mut expected = expected.clone();
                current.sort_by_cached_key(ToString::to_string);
                expected.sort_by_cached_key(ToString::to_string);
                if current != expected {
                    return Err(ProfileError::GroupChanged(group.clone()));
                }
                profiles.remove_group(group);
            }
            Self::Credential { target, auth, id } => {
                profiles.associate_credential(target, auth, *id);
            }
        }
        Ok(())
    }
}

fn require_group(profiles: &Profiles, group: &str) -> Result<(), ProfileError> {
    if profiles.has_group(group) {
        Ok(())
    } else {
        Err(ProfileError::MissingGroup(group.to_owned()))
    }
}
