//! Package manifests: the data an author writes, and the checks before anything is shown for
//! consent.
//!
//! A package is one TOML file. The adapter parses it into [`ManifestDto`] (unknown keys are
//! refused, because a package is written for one API version); [`Manifest::validate`] then
//! builds the ids, clamps the contributions, maps the `[settings]` table onto a schema, resolves
//! every template against what the package declares and refuses anything that breaks a limit.
//! Nothing here reads a file.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use nocterm_plugin_model::{IdError, PluginId, UserPlugin, UserPluginError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    API_VERSION, ApiVersion, CapabilityEntry, LimitError, VersionError, sanitize_display,
    template::ViewTemplate,
};

mod contribution;
mod declare;
mod settings;

pub use contribution::{ContributionDto, ContributionError};
pub use declare::{DeclareError, MAX_DECLARED, OptionalCapability};
pub use settings::{
    ChoiceOptionDto, DefaultDto, FieldDto, FieldKindName, GroupDto, SettingsDto, SettingsError,
};

/// Most contributions in one package.
pub const MAX_CONTRIBUTIONS: usize = 8;
/// Longest plugin name, in characters.
pub const MAX_NAME: usize = 48;
/// Longest description, in characters.
pub const MAX_DESCRIPTION: usize = 160;
/// Longest reason an author gives for a capability, in characters.
pub const MAX_REASON: usize = 160;

/// How a package runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    /// Views are templates the host interprets; the package has no code.
    #[default]
    Declarative,
}

/// The `api` key alone, read first, so a package written for a newer API is told so instead of
/// failing on keys this host does not know.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ManifestHeader {
    pub api: String,
}

impl ManifestHeader {
    /// The API version of the package, if this host can run it.
    pub fn check(&self, host: ApiVersion) -> Result<ApiVersion, ManifestError> {
        let api: ApiVersion = self.api.parse()?;
        if api.runs_on(host) {
            Ok(api)
        } else {
            Err(ManifestError::RequiresNewer { api })
        }
    }
}

/// A manifest as written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ManifestDto {
    pub api: String,
    #[serde(default)]
    pub runtime: Runtime,
    /// `[a-z0-9]+`; the host builds the plugin id `user-<publisher>-<id>`.
    pub publisher: String,
    /// `[a-z0-9]+`.
    pub id: String,
    /// The display name, `[A-Za-z0-9 ._-]`, at most 48 characters.
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub optional_capabilities: Vec<String>,
    /// Why a capability is wanted, by capability id. Shown as "Author says (unverified)".
    #[serde(default)]
    pub reasons: BTreeMap<String, String>,
    #[serde(default, rename = "contribution")]
    pub contributions: Vec<ContributionDto>,
    #[serde(default)]
    pub settings: Option<SettingsDto>,
}

/// What the host knows that a manifest is checked against.
#[derive(Clone, Copy, Debug)]
pub struct HostContext<'a> {
    pub api: ApiVersion,
    /// Names a package may not take, compared without case: system plugin names and the titles of
    /// built-in contributions and groups. "nocterm" is always refused.
    pub reserved_names: &'a [&'a str],
    /// Lets a declarative manifest list effect capabilities, which it can never use. Only
    /// [`HostContext::with_stand_in_runtime`] turns it on.
    effects_without_code: bool,
}

impl<'a> HostContext<'a> {
    /// The context of a host that runs declarative packages only.
    #[must_use]
    pub const fn new(api: ApiVersion, reserved_names: &'a [&'a str]) -> Self {
        Self {
            api,
            reserved_names,
            effects_without_code: false,
        }
    }

    /// A context for tests that drive effects through a stand-in runtime: a declarative
    /// manifest may then list effect capabilities, which the real declarative runtime never
    /// uses. Behind the `test-support` feature, so no production code can ask for it.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub const fn with_stand_in_runtime(mut self) -> Self {
        self.effects_without_code = true;
        self
    }
}

impl Default for HostContext<'_> {
    fn default() -> Self {
        Self::new(API_VERSION, &[])
    }
}

/// A manifest the host refuses before consent.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ManifestError {
    #[error("invalid api version")]
    Version(#[from] VersionError),
    #[error("Requires a newer Nocterm (API {api})")]
    RequiresNewer { api: ApiVersion },
    #[error("invalid id")]
    Id(#[from] IdError),
    #[error(
        "the name `{0}` is not allowed: use letters, digits, spaces, `.`, `_` and `-` and avoid the names the app uses"
    )]
    Name(String),
    #[error("a package has at most {MAX_CONTRIBUTIONS} contributions and needs at least one")]
    Contributions,
    #[error("a package value is outside its limits")]
    Limit(#[from] LimitError),
    #[error("capabilities")]
    Declare(#[from] DeclareError),
    #[error("contribution `{local}`")]
    Contribution {
        local: String,
        #[source]
        source: ContributionError,
    },
    #[error("settings")]
    Settings(#[from] SettingsError),
    #[error("the plugin is invalid")]
    Plugin(#[from] UserPluginError),
}

/// A checked package manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    api: ApiVersion,
    runtime: Runtime,
    publisher: String,
    plugin: UserPlugin,
    required: Vec<&'static CapabilityEntry>,
    optional: Vec<OptionalCapability>,
    reasons: BTreeMap<String, String>,
    views: BTreeMap<String, ViewTemplate>,
}

impl Manifest {
    /// Checks everything the host can check without running anything.
    pub fn validate(dto: &ManifestDto, host: &HostContext<'_>) -> Result<Self, ManifestError> {
        let api = ManifestHeader {
            api: dto.api.clone(),
        }
        .check(host.api)?;
        let id = PluginId::user(&dto.publisher, &dto.id)?;
        let name = checked_name(&dto.name, host)?;
        if dto.contributions.is_empty() || dto.contributions.len() > MAX_CONTRIBUTIONS {
            return Err(ManifestError::Contributions);
        }
        let declaration = declare::Declaration::resolve(
            &dto.capabilities,
            &dto.optional_capabilities,
            &dto.reasons,
        )?;
        match dto.runtime {
            Runtime::Declarative if host.effects_without_code => {}
            Runtime::Declarative => declaration.observation_only()?,
        }
        let schema = settings::schema(&id, dto.settings.as_ref())?;
        let built = contribution::build(&id, &dto.contributions, &declaration, &schema, host)?;
        let description = sanitize_display(&dto.description, MAX_DESCRIPTION);
        let plugin = UserPlugin::new(id, &name, &description, built.contributions, schema)?;
        Ok(Self {
            api,
            runtime: dto.runtime,
            publisher: dto.publisher.clone(),
            plugin,
            required: declaration.required,
            optional: declaration.optional,
            reasons: declaration.reasons,
            views: built.views,
        })
    }

    pub fn api(&self) -> ApiVersion {
        self.api
    }

    pub fn runtime(&self) -> Runtime {
        self.runtime
    }

    /// The publisher part of the id, which the host cannot verify.
    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    /// The plugin as the shell sees it: id, name, clamped contributions and settings schema.
    pub fn plugin(&self) -> &UserPlugin {
        &self.plugin
    }

    /// Capabilities the user must grant for the package to be installed.
    pub fn required(&self) -> &[&'static CapabilityEntry] {
        &self.required
    }

    /// Capabilities the user may grant at consent; without a grant they are unavailable.
    pub fn optional(&self) -> &[OptionalCapability] {
        &self.optional
    }

    /// The author's reason for a capability, sanitized, to be quoted as unverified.
    pub fn reason(&self, capability: &str) -> Option<&str> {
        self.reasons.get(capability).map(String::as_str)
    }

    /// The checked view of a contribution, by its local name.
    pub fn view(&self, local: &str) -> Option<&ViewTemplate> {
        self.views.get(local)
    }

    /// Every checked view with the local name of its contribution.
    pub fn views(&self) -> impl Iterator<Item = (&str, &ViewTemplate)> {
        self.views
            .iter()
            .map(|(local, view)| (local.as_str(), view))
    }
}

/// A name or title the host shows as it is: not blank, at most `MAX_NAME` characters, and only
/// ASCII letters, digits, space, `.`, `_` and `-`. No invisible or look-alike character can
/// pass, so what the reserved-name check compares is what the user reads.
pub(crate) fn is_plain(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

fn checked_name(name: &str, host: &HostContext<'_>) -> Result<String, ManifestError> {
    if !is_plain(name) || is_reserved(name, host) {
        return Err(ManifestError::Name(name.chars().take(MAX_NAME).collect()));
    }
    Ok(name.trim().to_string())
}

pub(crate) fn is_reserved(name: &str, host: &HostContext<'_>) -> bool {
    let name = name.trim();
    name.eq_ignore_ascii_case("nocterm")
        || host
            .reserved_names
            .iter()
            .any(|reserved| reserved.trim().eq_ignore_ascii_case(name))
}
