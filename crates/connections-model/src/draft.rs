//! Text drafts and checked values; defaults and identifiers are supplied by callers.
use nocterm_session::{
    Auth, CredentialId, CredentialIdParseError, DEFAULT_PORT, LaunchValidationError,
    SessionOptions, SessionOptionsError, ShellLaunch, Target,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthKind {
    #[default]
    Auto,
    Password,
    Key,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationField {
    Host,
    Port,
    User,
    Key,
    Credential,
    Arguments,
    Environment,
    Launch,
    Session,
    Description,
    IconColor,
    Country,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileValidationError {
    #[error("Enter a host name or address.")]
    MissingHost,
    #[error("The host is just the name or address, as in example.com.")]
    InvalidHost,
    #[error("`{0}` is not a port number.")]
    InvalidPort(String),
    #[error("Enter the user to sign in as.")]
    MissingUser,
    #[error("Choose the private key file to sign in with.")]
    MissingKey,
    #[error(transparent)]
    Credential(#[from] CredentialIdParseError),
    #[error("Arguments must be a JSON array of strings.")]
    Arguments(#[source] serde_json::Error),
    #[error("Environment must be a JSON object of string values.")]
    Environment(#[source] serde_json::Error),
    #[error(transparent)]
    Launch(#[from] LaunchValidationError),
    #[error(transparent)]
    Session(#[from] SessionOptionsError),
    #[error("`{0}` is not a colour such as #E95420.")]
    IconColor(String),
    #[error("`{0}` is not a two-letter country code such as DE.")]
    Country(String),
    #[error("Description must be at most 16384 bytes.")]
    Description,
}
impl ProfileValidationError {
    pub fn field(&self) -> ValidationField {
        match self {
            Self::MissingHost | Self::InvalidHost => ValidationField::Host,
            Self::InvalidPort(_) => ValidationField::Port,
            Self::MissingUser => ValidationField::User,
            Self::MissingKey => ValidationField::Key,
            Self::Credential(_) => ValidationField::Credential,
            Self::Arguments(_) => ValidationField::Arguments,
            Self::Environment(_) => ValidationField::Environment,
            Self::Launch(_) => ValidationField::Launch,
            Self::Session(_) => ValidationField::Session,
            Self::IconColor(_) => ValidationField::IconColor,
            Self::Country(_) => ValidationField::Country,
            Self::Description => ValidationField::Description,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataDraft {
    pub name: String,
    pub group: String,
    pub description: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndpointDraft {
    pub host: String,
    pub port: String,
    pub user: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthenticationDraft {
    pub kind: AuthKind,
    pub key_path: String,
    pub credential: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchDraft {
    pub override_launch: bool,
    pub integration: bool,
    pub program: String,
    pub args: String,
    pub cwd: String,
    pub env: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppearanceDraft {
    pub icon: String,
    pub icon_color: String,
    pub country: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileDraft {
    pub metadata: MetadataDraft,
    pub endpoint: EndpointDraft,
    pub authentication: AuthenticationDraft,
    pub options: SessionOptions,
    pub launch: LaunchDraft,
    pub appearance: AppearanceDraft,
}
#[derive(Clone, Debug, Default)]
pub struct EditorDefaults {
    pub user: Option<String>,
    pub home: Option<PathBuf>,
}
/// Checked profile parts, independent of the caller's identifier format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedProfile {
    pub name: String,
    pub description: String,
    pub group: Option<String>,
    pub target: Target,
    pub auth: Auth,
    pub credential: Option<CredentialId>,
    pub options: SessionOptions,
    pub launch: Option<ShellLaunch>,
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    pub country: Option<String>,
}
impl ValidatedProfile {
    /// Attach the supplied persistent identity without introducing entropy or UI rules.
    pub fn into_profile(self, id: crate::profiles::ProfileId) -> crate::profiles::Profile {
        crate::profiles::Profile {
            id,
            name: self.name,
            description: self.description,
            options: self.options,
            target: self.target,
            auth: self.auth,
            group: self.group,
            credential: self.credential,
            launch: self.launch,
            icon: self.icon,
            icon_color: self.icon_color,
            country: self.country,
        }
    }
}
impl ProfileDraft {
    pub fn validate(
        &self,
        defaults: &EditorDefaults,
    ) -> Result<ValidatedProfile, ProfileValidationError> {
        let target = self.endpoint.validate(defaults.user.as_deref())?;
        let auth = self.authentication.validate(defaults.home.as_deref())?;
        let credential = optional(&self.authentication.credential)
            .map(|id| id.parse())
            .transpose()?;
        let launch = self.launch.validate()?;
        self.options.validate()?;
        let appearance = self.appearance.validate()?;
        if self.metadata.description.len() > 16384 {
            return Err(ProfileValidationError::Description);
        }
        Ok(ValidatedProfile {
            name: optional(&self.metadata.name).unwrap_or_else(|| target.host.clone()),
            description: self.metadata.description.trim().to_owned(),
            group: optional(&self.metadata.group),
            target,
            auth,
            credential,
            options: self.options.clone(),
            launch,
            icon: appearance.icon,
            icon_color: appearance.icon_color,
            country: appearance.country,
        })
    }
}
impl EndpointDraft {
    fn validate(&self, default_user: Option<&str>) -> Result<Target, ProfileValidationError> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(ProfileValidationError::MissingHost);
        }
        if host.contains(char::is_whitespace) || host.contains('@') {
            return Err(ProfileValidationError::InvalidHost);
        }
        let port = match self.port.trim() {
            "" => DEFAULT_PORT,
            port => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| ProfileValidationError::InvalidPort(port.to_owned()))?,
        };
        let user = optional(&self.user)
            .or_else(|| default_user.map(str::to_owned))
            .ok_or(ProfileValidationError::MissingUser)?;
        Ok(Target::new(user, host, port))
    }
}
impl AuthenticationDraft {
    fn validate(&self, home: Option<&Path>) -> Result<Auth, ProfileValidationError> {
        Ok(match self.kind {
            AuthKind::Auto => Auth::Auto,
            AuthKind::Password => Auth::Password,
            AuthKind::Key => {
                let path = optional(&self.key_path).ok_or(ProfileValidationError::MissingKey)?;
                Auth::Key {
                    path: expand_home(&path, home),
                }
            }
        })
    }
}
impl LaunchDraft {
    fn validate(&self) -> Result<Option<ShellLaunch>, ProfileValidationError> {
        if !self.override_launch && self.cwd.trim().is_empty() {
            return Ok(None);
        }
        let launch = ShellLaunch {
            program: self
                .override_launch
                .then(|| optional(&self.program))
                .flatten(),
            args: if self.override_launch && !self.args.trim().is_empty() {
                serde_json::from_str(&self.args).map_err(ProfileValidationError::Arguments)?
            } else {
                Vec::new()
            },
            cwd: optional(&self.cwd),
            env: if self.override_launch && !self.env.trim().is_empty() {
                serde_json::from_str(&self.env).map_err(ProfileValidationError::Environment)?
            } else {
                Default::default()
            },
            integration: !self.override_launch || self.integration,
        };
        launch.validate()?;
        Ok(Some(launch))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedAppearance {
    icon: Option<String>,
    icon_color: Option<String>,
    country: Option<String>,
}
impl AppearanceDraft {
    fn validate(&self) -> Result<ValidatedAppearance, ProfileValidationError> {
        let icon = optional(&self.icon);
        let icon_color = optional(&self.icon_color)
            .map(|typed| {
                let color = if typed.starts_with('#') {
                    typed.clone()
                } else {
                    format!("#{typed}")
                };
                if !valid_color(&color) {
                    return Err(ProfileValidationError::IconColor(typed));
                }
                Ok(color.to_ascii_uppercase())
            })
            .transpose()?;
        let country = optional(&self.country)
            .map(|code| {
                let lower = code.to_ascii_lowercase();
                crate::observation::CountryCode::new(&lower)
                    .map_err(|_| ProfileValidationError::Country(code))?;
                Ok::<_, ProfileValidationError>(lower)
            })
            .transpose()?;
        Ok(ValidatedAppearance {
            icon,
            icon_color,
            country,
        })
    }
}
fn optional(value: &str) -> Option<String> {
    Some(value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}
pub fn valid_color(color: &str) -> bool {
    color
        .strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests;
