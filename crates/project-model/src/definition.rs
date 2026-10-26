use nocterm_session::{HostAddress, MAX_JUMP_HOSTS, Target};
use serde::{Deserialize, Serialize};

use crate::{ProjectId, ProjectTarget, ProjectValidationError, RootPath};

/// Original ordinary connection description, including its ordered jump route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostLocator {
    Local,
    Ssh {
        profile: String,
        #[serde(deserialize_with = "deserialize_target")]
        target: Target,
        #[serde(deserialize_with = "deserialize_route")]
        via: Vec<Target>,
    },
    SshDirect {
        #[serde(deserialize_with = "deserialize_target")]
        target: Target,
        #[serde(deserialize_with = "deserialize_route")]
        via: Vec<Target>,
    },
}

/// Stable saved-parent grouping, independent of the currently focused terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentConnectionKey(ParentIdentity);

#[derive(Clone, Debug, PartialEq, Eq)]
enum ParentIdentity {
    Local,
    Profile(String),
    Direct(HostAddress),
}

// Session Target remains lenient for its existing callers. Saved folder
// definitions reject misspelled host data rather than silently changing it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRecord {
    host: String,
    user: String,
    #[serde(default = "default_port")]
    port: u16,
}

fn default_port() -> u16 {
    nocterm_session::DEFAULT_PORT
}

impl From<TargetRecord> for Target {
    fn from(value: TargetRecord) -> Self {
        Self::new(value.user, value.host, value.port)
    }
}

fn deserialize_target<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Target, D::Error> {
    TargetRecord::deserialize(deserializer).map(Into::into)
}

fn deserialize_route<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Target>, D::Error> {
    Vec::<TargetRecord>::deserialize(deserializer)
        .map(|targets| targets.into_iter().map(Into::into).collect())
}

impl HostLocator {
    pub fn validate(&self) -> Result<(), ProjectValidationError> {
        match self {
            Self::Local => Ok(()),
            Self::Ssh {
                profile,
                target,
                via,
            } if valid_text(profile, 128)
                && via.len() <= MAX_JUMP_HOSTS
                && std::iter::once(target).chain(via).all(valid_target) =>
            {
                Ok(())
            }
            Self::SshDirect { target, via }
                if via.len() <= MAX_JUMP_HOSTS
                    && std::iter::once(target).chain(via).all(valid_target) =>
            {
                Ok(())
            }
            Self::Ssh { .. } | Self::SshDirect { .. } => Err(ProjectValidationError::InvalidHost),
        }
    }

    pub fn address(&self) -> Option<HostAddress> {
        match self {
            Self::Local => None,
            Self::Ssh { target, via, .. } | Self::SshDirect { target, via } => {
                Some(HostAddress::new(target.clone(), via.clone()))
            }
        }
    }

    pub fn parent_key(&self) -> Result<ParentConnectionKey, ProjectValidationError> {
        self.validate()?;
        Ok(ParentConnectionKey(match self {
            Self::Local => ParentIdentity::Local,
            Self::Ssh { profile, .. } => ParentIdentity::Profile(profile.clone()),
            Self::SshDirect { target, via } => {
                ParentIdentity::Direct(HostAddress::new(target.clone(), via.clone()))
            }
        }))
    }
}

fn valid_target(target: &Target) -> bool {
    valid_text(&target.host, 253)
        && !target.host.contains(char::is_whitespace)
        && valid_text(&target.user, 128)
        && !target.user.contains(char::is_whitespace)
        && target.port != 0
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

/// Inert provenance only. Imports pass through the same definition validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectSource {
    Manual,
    Imported {
        provider: String,
        /// Opaque non-secret deployment key, not a trusted endpoint or token.
        deployment: String,
        /// Opaque non-secret account key, distinct across provider accounts.
        account: String,
        external_id: String,
    },
}

impl ProjectSource {
    fn validate(&self) -> Result<(), ProjectValidationError> {
        match self {
            Self::Manual => Ok(()),
            Self::Imported {
                provider,
                deployment,
                account,
                external_id,
            } if valid_text(provider, 128)
                && valid_text(deployment, 512)
                && valid_text(account, 512)
                && valid_text(external_id, 512) =>
            {
                Ok(())
            }
            Self::Imported { .. } => Err(ProjectValidationError::InvalidSource),
        }
    }
}

/// Editable plain data. Conversion checks structural rules without native I/O.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectDraft {
    pub id: ProjectId,
    pub name: String,
    pub target: ProjectTarget,
    pub source: ProjectSource,
    /// Supplied Unix timestamp; the domain never reads a clock.
    pub updated: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedDraft {
    Current(CurrentDraft),
    Legacy(LegacyDraft),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentDraft {
    id: ProjectId,
    name: String,
    target: ProjectTarget,
    source: ProjectSource,
    updated: u64,
    #[serde(default, rename = "policy")]
    _legacy: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDraft {
    id: ProjectId,
    name: String,
    host: HostLocator,
    root: RootPath,
    source: ProjectSource,
    updated: u64,
    #[serde(default, rename = "policy")]
    _legacy: Option<serde::de::IgnoredAny>,
}

impl<'de> Deserialize<'de> for ProjectDraft {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match SavedDraft::deserialize(deserializer)? {
            SavedDraft::Current(saved) => Self {
                id: saved.id,
                name: saved.name,
                target: saved.target,
                source: saved.source,
                updated: saved.updated,
            },
            SavedDraft::Legacy(saved) => Self {
                id: saved.id,
                name: saved.name,
                target: ProjectTarget::HostFolder {
                    host: saved.host,
                    root: saved.root,
                },
                source: saved.source,
                updated: saved.updated,
            },
        })
    }
}

/// A checked saved folder and its original connection description.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ProjectDefinition(ProjectDraft);

impl<'de> Deserialize<'de> for ProjectDefinition {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ProjectDraft::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl ProjectDefinition {
    pub fn id(&self) -> ProjectId {
        self.0.id
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn target(&self) -> &ProjectTarget {
        &self.0.target
    }

    pub fn host_folder(&self) -> Option<(&HostLocator, &RootPath)> {
        self.0.target.host_folder()
    }

    pub fn source(&self) -> &ProjectSource {
        &self.0.source
    }

    pub fn updated(&self) -> u64 {
        self.0.updated
    }

    pub fn to_draft(&self) -> ProjectDraft {
        self.0.clone()
    }
}

impl TryFrom<ProjectDraft> for ProjectDefinition {
    type Error = ProjectValidationError;

    fn try_from(draft: ProjectDraft) -> Result<Self, Self::Error> {
        if !valid_text(&draft.name, 128) {
            return Err(ProjectValidationError::InvalidName);
        }
        draft.target.validate()?;
        draft.source.validate()?;
        Ok(Self(draft))
    }
}

impl From<ProjectDefinition> for ProjectDraft {
    fn from(value: ProjectDefinition) -> Self {
        value.0
    }
}
