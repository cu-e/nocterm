//! Actual published control facts share independent command, configuration and mode owners.
mod choices;
mod commands;
mod configuration;
mod facts;
use super::super::{
    bundle::{self, Body, BundleOwner, BundleSlot},
    slots::*,
};
use crate::model::{
    controls::ConfigOption,
    ids::ModeId,
    native::NativeOwned,
    notice::AdvertisedCommand,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, Publication},
        graph::Count,
    },
};
pub use choices::{ConfigChoiceSlot, ConfigGroupSlot, SelectChoicesSlot};
pub use commands::CommandSlot;
use commands::CommandsSlot;
use configuration::ConfigurationSlot;
pub use configuration::{CategorySlot, ConfigKindSlot, ConfigOptionSlot, ConfigSelectionSlot};
use facts::ControlFacts;
pub struct LiveControls {
    commands: Option<BundleSlot<CommandsSlot>>,
    configuration: Option<BundleSlot<ConfigurationSlot>>,
    mode: Option<BundleSlot<IdSlot<ModeId>>>,
    commands_revision: u64,
    info: Option<BundleSlot<ControlFacts<crate::connection::AgentInfo>>>,
    modes: Option<BundleSlot<ControlFacts<crate::model::controls::Modes>>>,
}
impl LiveControls {
    pub fn commands(&self) -> &[CommandSlot] {
        self.commands.as_ref().map_or(&[], |value| &value.body().0)
    }
    pub fn info(&self) -> Option<PayloadGuard<'_, crate::connection::AgentInfo>> {
        self.info.as_ref().map(|value| value.body().read())
    }
    pub fn modes(&self) -> Option<PayloadGuard<'_, crate::model::controls::Modes>> {
        self.modes.as_ref().map(|value| value.body().read())
    }
    pub fn configuration(&self) -> &[ConfigOptionSlot] {
        self.configuration
            .as_ref()
            .map_or(&[], |value| &value.body().0)
    }
    pub fn mode(&self) -> Option<PayloadGuard<'_, ModeId>> {
        self.mode.as_ref().map(|value| value.body().read())
    }
    pub fn commands_revision(&self) -> u64 {
        self.commands_revision
    }
    pub(in crate::history::physical) fn replace_opened(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<crate::model::session::SessionOpened>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let crate::model::session::SessionOpened {
            session,
            modes,
            configuration,
        } = source.take();
        drop(session);
        let configuration = match configuration {
            Some(values) => {
                let mut builder = SlabBuilder::new(role)?;
                let value = ConfigurationSlot(builder.map_in(
                    values,
                    source.pool(),
                    ConfigOptionSlot::new,
                )?);
                Some(BundleSlot::new(BundleOwner::build(
                    value, builder, &original,
                )?))
            }
            None => None,
        };
        let mode = modes
            .as_ref()
            .map(|value| bundle::part(role, &original, IdSlot::mode(value.current.clone())))
            .transpose()?;
        let modes = match modes {
            Some(value) => {
                let policy = crate::history::source::modes_policy(&value)?;
                Some(ControlFacts::from_source(
                    role,
                    value,
                    source.pool(),
                    policy,
                )?)
            }
            None => None,
        };
        bundle::part(
            role,
            &original,
            Self {
                commands: previous.and_then(|old| old.commands.as_ref().map(BundleSlot::alias)),
                configuration,
                mode,
                commands_revision: previous.map_or(0, |old| old.commands_revision),
                info: previous.and_then(|old| old.info.as_ref().map(BundleSlot::alias)),
                modes,
            },
        )
    }
    pub(in crate::history::physical) fn replace_info(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<crate::connection::AgentInfo>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let info = ControlFacts::prepare(role, value, 0)?;
        let original = info.original_credit().clone();
        bundle::part(
            role,
            &original,
            Self {
                info: Some(info),
                modes: previous.and_then(|old| old.modes.as_ref().map(BundleSlot::alias)),
                commands: previous.and_then(|old| old.commands.as_ref().map(BundleSlot::alias)),
                configuration: previous
                    .and_then(|old| old.configuration.as_ref().map(BundleSlot::alias)),
                mode: previous.and_then(|old| old.mode.as_ref().map(BundleSlot::alias)),
                commands_revision: previous.map_or(0, |old| old.commands_revision),
            },
        )
    }
    pub(in crate::history::physical) fn replace_commands(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<Vec<AdvertisedCommand>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let commands =
            CommandsSlot(builder.map_in(source.take(), source.pool(), CommandSlot::new)?);
        let commands = BundleSlot::new(BundleOwner::build(commands, builder, &original)?);
        bundle::part(
            role,
            &original,
            Self {
                info: previous.and_then(|old| old.info.as_ref().map(BundleSlot::alias)),
                modes: previous.and_then(|old| old.modes.as_ref().map(BundleSlot::alias)),
                commands: Some(commands),
                configuration: previous
                    .and_then(|old| old.configuration.as_ref().map(BundleSlot::alias)),
                mode: previous.and_then(|old| old.mode.as_ref().map(BundleSlot::alias)),
                commands_revision: previous
                    .map_or(0, |old| old.commands_revision)
                    .wrapping_add(1),
            },
        )
    }
    pub(in crate::history::physical) fn replace_configuration(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<Vec<ConfigOption>>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mut builder = SlabBuilder::new(role)?;
        let configuration = ConfigurationSlot(builder.map_in(
            source.take(),
            source.pool(),
            ConfigOptionSlot::new,
        )?);
        let configuration = BundleSlot::new(BundleOwner::build(configuration, builder, &original)?);
        bundle::part(
            role,
            &original,
            Self {
                info: previous.and_then(|old| old.info.as_ref().map(BundleSlot::alias)),
                modes: previous.and_then(|old| old.modes.as_ref().map(BundleSlot::alias)),
                commands: previous.and_then(|old| old.commands.as_ref().map(BundleSlot::alias)),
                configuration: Some(configuration),
                mode: previous.and_then(|old| old.mode.as_ref().map(BundleSlot::alias)),
                commands_revision: previous.map_or(0, |old| old.commands_revision),
            },
        )
    }
    pub(in crate::history::physical) fn replace_mode(
        role: &ConstructionRole,
        previous: Option<&Self>,
        value: NativeOwned<ModeId>,
    ) -> Result<BundleSlot<Self>, PhysicalError> {
        let mut source = NativeSource::new(value, role)?;
        let original = source.original().clone();
        let mode = bundle::part(role, &original, IdSlot::mode(source.take()))?;
        bundle::part(
            role,
            &original,
            Self {
                info: previous.and_then(|old| old.info.as_ref().map(BundleSlot::alias)),
                modes: previous.and_then(|old| old.modes.as_ref().map(BundleSlot::alias)),
                commands: previous.and_then(|old| old.commands.as_ref().map(BundleSlot::alias)),
                configuration: previous
                    .and_then(|old| old.configuration.as_ref().map(BundleSlot::alias)),
                mode: Some(mode),
                commands_revision: previous.map_or(0, |old| old.commands_revision),
            },
        )
    }
}
impl Body for LiveControls {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if let Some(value) = &self.info {
            value.count(count)?;
        }
        if let Some(value) = &self.modes {
            value.count(count)?;
        }
        if let Some(value) = &self.commands {
            value.count(count)?;
        }
        if let Some(value) = &self.configuration {
            value.count(count)?;
        }
        if let Some(value) = &self.mode {
            value.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        if let Some(value) = &self.info {
            value.cleanup(permit);
        }
        if let Some(value) = &self.modes {
            value.cleanup(permit);
        }
        if let Some(value) = &self.commands {
            value.cleanup(permit);
        }
        if let Some(value) = &self.configuration {
            value.cleanup(permit);
        }
        if let Some(value) = &self.mode {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        if let Some(value) = &self.info {
            value.adopt(publication);
        }
        if let Some(value) = &self.modes {
            value.adopt(publication);
        }
        if let Some(value) = &self.commands {
            value.adopt(publication);
        }
        if let Some(value) = &self.configuration {
            value.adopt(publication);
        }
        if let Some(value) = &self.mode {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        if let Some(value) = &self.info {
            value.retire();
        }
        if let Some(value) = &self.modes {
            value.retire();
        }
        if let Some(value) = &self.commands {
            value.retire();
        }
        if let Some(value) = &self.configuration {
            value.retire();
        }
        if let Some(value) = &self.mode {
            value.retire();
        }
    }
}

#[cfg(test)]
mod tests;
