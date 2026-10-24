//! The contributions of a package: clamped for the shell, with a checked view each.

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use nocterm_plugin_model::{ContributionId, IdError, PluginId, SettingsSchema, UserContribution};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{HostContext, ManifestError, declare::Declaration, is_plain, is_reserved};
use crate::{
    Kind, LimitError, LocalId, PluginIcon, Zone,
    template::{ViewDto, ViewTemplate, ViewTemplateError},
};

/// A contribution as written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ContributionDto {
    /// `[a-z0-9-]{1,32}`; the host builds the id `<plugin id>.<local>`.
    pub local: String,
    pub kind: Option<Kind>,
    pub title: String,
    #[serde(default)]
    pub icon: Option<PluginIcon>,
    /// Where it sits once the user places it. Default: right.
    #[serde(default)]
    pub zone: Option<Zone>,
    #[serde(default)]
    pub order: u16,
    pub view: ViewDto,
}

/// A contribution the host refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ContributionError {
    #[error("invalid local name")]
    Local(#[from] LimitError),
    #[error("invalid id")]
    Id(#[from] IdError),
    #[error("it needs a `kind` of widget or panel")]
    NoKind,
    #[error(
        "its title must be 1 to 48 letters, digits, spaces, `.`, `_` or `-`, and not a name the app reserves"
    )]
    Title,
    #[error("it is declared twice")]
    Duplicate,
    #[error("its view")]
    View(#[from] ViewTemplateError),
}

pub(super) struct Built {
    pub(super) contributions: Vec<UserContribution>,
    pub(super) views: BTreeMap<String, ViewTemplate>,
}

pub(super) fn build(
    plugin: &PluginId,
    dtos: &[ContributionDto],
    declaration: &Declaration,
    schema: &SettingsSchema,
    host: &HostContext<'_>,
) -> Result<Built, ManifestError> {
    let observing = declaration.observing();
    let mut contributions = Vec::with_capacity(dtos.len());
    let mut views = BTreeMap::new();
    for dto in dtos {
        let fail = |source: ContributionError| ManifestError::Contribution {
            local: dto.local.chars().take(32).collect(),
            source,
        };
        let local = LocalId::parse(&dto.local).map_err(|e| fail(e.into()))?;
        let kind = dto.kind.ok_or_else(|| fail(ContributionError::NoKind))?;
        if !is_plain(&dto.title) {
            return Err(fail(ContributionError::Title));
        }
        let id = ContributionId::in_plugin(plugin, local.as_str()).map_err(|e| fail(e.into()))?;
        let view = ViewTemplate::compile(&dto.view, &observing, schema)
            .map_err(|e| fail(ContributionError::View(e)))?;
        let clamped = UserContribution::clamp(
            id,
            kind.to_model(),
            &dto.title,
            dto.icon.unwrap_or(PluginIcon::Plug).to_icon(),
            dto.zone.unwrap_or(Zone::Right).to_model(),
            dto.order,
        )?;
        // The check that matters runs on the title the shell will show.
        if is_reserved(clamped.title(), host) {
            return Err(fail(ContributionError::Title));
        }
        if views.insert(String::from(local.as_str()), view).is_some() {
            return Err(fail(ContributionError::Duplicate));
        }
        contributions.push(clamped);
    }
    Ok(Built {
        contributions,
        views,
    })
}
