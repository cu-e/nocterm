//! Named neutral copies share the same prepaid constructors across adapters and fakes.
use super::{NativeList, NativeOwned};
use crate::{
    connection::AgentInfo,
    model::{
        auth::*,
        controls::*,
        extensions::Extensions,
        json::deferred::{DecodeLedger, JsonDecodeError},
        session::{EndOfTurn, PromptOutcome, SessionOpened},
    },
};
mod content;
mod frames;
mod notices;
mod physical;
mod tools;
pub struct NativeCopies<'a> {
    ledger: &'a DecodeLedger,
}
impl<'a> NativeCopies<'a> {
    pub fn new(ledger: &'a DecodeLedger) -> Self {
        Self { ledger }
    }
    pub fn physical_content(
        &self,
        value: &crate::history::physical::ContentSlot,
    ) -> Result<NativeOwned<crate::model::content::Content>, JsonDecodeError> {
        physical::PhysicalContentCopies::new(self).copy(value)
    }
    pub fn text(&self, value: &str) -> Result<NativeOwned<String>, JsonDecodeError> {
        let allocation = self.ledger.reserve(value.len())?;
        let text = String::from(value);
        if text.capacity() != value.len() {
            return Err(JsonDecodeError::CapacityProfile);
        }
        Ok(NativeOwned::from_parts(text, allocation))
    }
    pub fn optional_text(
        &self,
        value: Option<&str>,
    ) -> Result<NativeOwned<Option<String>>, JsonDecodeError> {
        match value {
            Some(value) => Ok(NativeOwned::supplied(self.text(value)?)),
            None => NativeOwned::absent(self.ledger),
        }
    }
    pub fn optional_path(
        &self,
        value: Option<&std::path::Path>,
    ) -> Result<NativeOwned<Option<std::path::PathBuf>>, JsonDecodeError> {
        match value {
            Some(path) => Ok(NativeOwned::supplied(
                super::path::PathConstruction::new(self.ledger).copy(path)?,
            )),
            None => NativeOwned::absent(self.ledger),
        }
    }
    pub fn config_value(
        &self,
        value: &crate::session_config::ConfigValue,
    ) -> Result<NativeOwned<crate::session_config::ConfigValue>, JsonDecodeError> {
        match value {
            crate::session_config::ConfigValue::Flag(flag) => {
                crate::model::native::choices::flag(*flag, self.ledger)
            }
            crate::session_config::ConfigValue::Value(text) => {
                Ok(crate::model::native::choices::text_value(self.text(text)?))
            }
        }
    }
    pub fn extensions(
        &self,
        value: Option<&Extensions>,
    ) -> Result<NativeOwned<Option<Extensions>>, JsonDecodeError> {
        frames::Frames::copy(self.ledger, value)
    }
    pub fn json(
        &self,
        value: &crate::model::json::OwnedJson,
    ) -> Result<NativeOwned<crate::model::json::OwnedJson>, JsonDecodeError> {
        frames::Frames::json(self.ledger, value)
    }
    pub fn content(
        &self,
        value: &crate::model::content::Content,
    ) -> Result<NativeOwned<crate::model::content::Content>, JsonDecodeError> {
        content::ContentCopies::new(self).copy(value)
    }
    pub fn tool(
        &self,
        value: &crate::model::tools::ToolRecord,
    ) -> Result<NativeOwned<crate::model::tools::ToolRecord>, JsonDecodeError> {
        tools::ToolCopies::new(self).record(value)
    }
    pub fn client_display(
        &self,
        value: &crate::model::tools::ClientDisplay,
    ) -> Result<NativeOwned<crate::model::tools::ClientDisplay>, JsonDecodeError> {
        tools::ToolCopies::new(self).display(value)
    }
    pub fn patch(
        &self,
        value: &crate::model::tools::ToolPatch,
    ) -> Result<NativeOwned<crate::model::tools::ToolPatch>, JsonDecodeError> {
        tools::ToolCopies::new(self).patch(value)
    }
    pub fn notice(
        &self,
        value: &crate::model::notice::SessionNotice,
    ) -> Result<NativeOwned<crate::model::notice::SessionNotice>, JsonDecodeError> {
        notices::NoticeCopies::new(self).notice(value)
    }
    pub fn permission(
        &self,
        value: &crate::model::auth::ProviderPermission,
    ) -> Result<NativeOwned<crate::model::auth::ProviderPermission>, JsonDecodeError> {
        notices::NoticeCopies::new(self).permission(value)
    }
    pub fn configurations(
        &self,
        values: &[ConfigOption],
        override_current: Option<(&str, &str)>,
    ) -> Result<NativeOwned<Vec<ConfigOption>>, JsonDecodeError> {
        let mut output = NativeList::new(self.ledger, values.len())?;
        for value in values {
            output.push(self.option(value, override_current)?)?;
        }
        Ok(output.finish())
    }
    pub fn option(
        &self,
        value: &ConfigOption,
        override_current: Option<(&str, &str)>,
    ) -> Result<NativeOwned<ConfigOption>, JsonDecodeError> {
        let kind = match &value.kind {
            ConfigKind::Boolean(value) => {
                crate::model::native::controls::boolean(*value, self.ledger)?
            }
            ConfigKind::Unsupported { label } => {
                crate::model::native::controls::unsupported(self.text(label)?)
            }
            ConfigKind::Select { current, choices } => {
                let current = override_current
                    .filter(|(id, _)| *id == value.id.as_str())
                    .map_or(current.as_str(), |(_, value)| value);
                crate::model::native::controls::select(
                    crate::model::native::controls::config_choice_id(self.text(current)?)?,
                    self.choices(choices)?,
                )?
            }
        };
        crate::model::native::controls::config_option(
            crate::model::native::controls::config_id(self.text(value.id.as_str())?)?,
            self.text(&value.name)?,
            self.optional_text(value.description.as_deref())?,
            self.category(value.category.as_ref())?,
            kind,
            self.extensions(value.extensions.as_ref())?,
        )
    }
    fn choices(
        &self,
        value: &SelectChoices,
    ) -> Result<NativeOwned<SelectChoices>, JsonDecodeError> {
        match value {
            SelectChoices::Flat(values) => Ok(crate::model::native::controls::flat(
                self.flat_choices(values)?,
            )),
            SelectChoices::Groups(values) => {
                let mut groups = NativeList::new(self.ledger, values.len())?;
                for value in values {
                    groups.push(crate::model::native::controls::config_group(
                        crate::model::native::controls::config_group_id(
                            self.text(value.id.as_str())?,
                        )?,
                        self.text(&value.name)?,
                        self.flat_choices(&value.choices)?,
                        self.extensions(value.extensions.as_ref())?,
                    )?)?;
                }
                Ok(crate::model::native::controls::groups(groups.finish()))
            }
        }
    }
    fn flat_choices(
        &self,
        values: &[ConfigChoice],
    ) -> Result<NativeOwned<Vec<ConfigChoice>>, JsonDecodeError> {
        let mut output = NativeList::new(self.ledger, values.len())?;
        for value in values {
            output.push(crate::model::native::controls::config_choice(
                crate::model::native::controls::config_choice_id(self.text(value.id.as_str())?)?,
                self.text(&value.name)?,
                self.optional_text(value.description.as_deref())?,
                self.extensions(value.extensions.as_ref())?,
            )?)?;
        }
        Ok(output.finish())
    }
    fn category(
        &self,
        value: Option<&ConfigCategory>,
    ) -> Result<NativeOwned<Option<ConfigCategory>>, JsonDecodeError> {
        let Some(value) = value else {
            return NativeOwned::absent(self.ledger);
        };
        let known = match value {
            ConfigCategory::Mode => "mode",
            ConfigCategory::Model => "model",
            ConfigCategory::ModelConfig => "model_config",
            ConfigCategory::ThoughtLevel => "thought_level",
            ConfigCategory::Other(value) => {
                return Ok(NativeOwned::supplied(
                    crate::model::native::controls::other(self.text(value)?),
                ));
            }
        };
        Ok(NativeOwned::supplied(
            crate::model::native::controls::known(known, self.ledger)?
                .ok_or(JsonDecodeError::ConstructionOrder)?,
        ))
    }
    fn modes(&self, value: &Modes) -> Result<NativeOwned<Modes>, JsonDecodeError> {
        let mut choices = NativeList::new(self.ledger, value.choices.len())?;
        for value in &value.choices {
            choices.push(crate::model::native::controls::mode_choice(
                crate::model::native::controls::mode_id(self.text(value.id.as_str())?)?,
                self.text(&value.name)?,
                self.optional_text(value.description.as_deref())?,
                self.extensions(value.extensions.as_ref())?,
            )?)?;
        }
        crate::model::native::controls::modes(
            crate::model::native::controls::mode_id(self.text(value.current.as_str())?)?,
            choices.finish(),
            self.extensions(value.extensions.as_ref())?,
        )
    }
    pub fn opened(
        &self,
        value: &SessionOpened,
    ) -> Result<NativeOwned<SessionOpened>, JsonDecodeError> {
        self.opened_fields(
            value.session.as_str(),
            value.modes.as_ref(),
            value.configuration.as_deref(),
        )
    }
    pub fn opened_fields(
        &self,
        session: &str,
        modes: Option<&Modes>,
        configuration: Option<&[ConfigOption]>,
    ) -> Result<NativeOwned<SessionOpened>, JsonDecodeError> {
        let modes = match modes {
            Some(value) => NativeOwned::supplied(self.modes(value)?),
            None => NativeOwned::absent(self.ledger)?,
        };
        let configuration = match configuration {
            Some(value) => NativeOwned::supplied(self.configurations(value, None)?),
            None => NativeOwned::absent(self.ledger)?,
        };
        crate::model::native::session::opened(
            crate::model::native::controls::session_id(self.text(session)?)?,
            modes,
            configuration,
        )
    }
    pub fn prompt(
        &self,
        value: &crate::history::physical::DispatchPayload,
    ) -> Result<NativeOwned<crate::model::session::PromptRequest>, JsonDecodeError> {
        let context = value.context();
        let history = value.history();
        let prompt = value.prompt();
        let count = 1usize
            .checked_add(prompt.images().len())
            .and_then(|count| {
                count.checked_add(usize::from(context.is_some()) + usize::from(history.is_some()))
            })
            .ok_or(JsonDecodeError::Arithmetic)?;
        let mut output = NativeList::new(self.ledger, count)?;
        for value in context.iter().chain(history.iter()) {
            output.push(crate::model::native::content::from_text(
                crate::model::native::content::text_content(
                    self.text(value)?,
                    NativeOwned::absent(self.ledger)?,
                    NativeOwned::absent(self.ledger)?,
                )?,
            ))?;
        }
        output.push(crate::model::native::content::from_text(
            crate::model::native::content::text_content(
                self.text(&prompt.text())?,
                NativeOwned::absent(self.ledger)?,
                NativeOwned::absent(self.ledger)?,
            )?,
        ))?;
        for value in prompt.images() {
            output.push(self.physical_content(value)?)?;
        }
        crate::model::native::session::request(
            crate::model::native::controls::session_id(self.text(value.session().as_str())?)?,
            output.finish(),
        )
    }
    pub fn info(&self, value: &AgentInfo) -> Result<NativeOwned<AgentInfo>, JsonDecodeError> {
        let capabilities = &value.capabilities;
        let capabilities = crate::model::native::session::capabilities(
            capabilities.content.image,
            capabilities.content.audio,
            capabilities.content.embedded_resources,
            capabilities.restore,
            capabilities.fork,
            capabilities.close,
            self.ledger,
        )?;
        let mut auth = NativeList::new(self.ledger, value.auth_methods.len())?;
        for method in &value.auth_methods {
            auth.push(self.auth_method(method)?)?;
        }
        crate::model::native::session::agent_info(
            self.text(&value.name)?,
            self.text(&value.version)?,
            capabilities,
            auth.finish(),
        )
    }
    fn auth_method(&self, value: &AuthMethod) -> Result<NativeOwned<AuthMethod>, JsonDecodeError> {
        let flow = match &value.flow {
            AuthFlow::AgentManaged => crate::model::native::auth::agent_managed(self.ledger)?,
            AuthFlow::TerminalSetup(setup) => {
                let mut args = NativeList::new(self.ledger, setup.args.len())?;
                for arg in &setup.args {
                    args.push(self.text(arg)?)?;
                }
                let mut env = NativeList::new(self.ledger, setup.env.len())?;
                for (key, value) in setup.env.iter() {
                    env.push(crate::model::native::auth::environment_pair(
                        self.text(key)?,
                        self.text(value)?,
                    )?)?;
                }
                crate::model::native::auth::terminal_setup_flow(
                    crate::model::native::auth::terminal_setup(
                        args.finish(),
                        crate::model::native::auth::environment(env.finish())?,
                    )?,
                )
            }
        };
        crate::model::native::auth::auth_method(
            crate::model::native::auth::auth_method_id(self.text(value.id.as_str())?)?,
            self.text(&value.label)?,
            self.optional_text(value.description.as_deref())?,
            flow,
        )
    }
    pub fn outcome(
        &self,
        value: &PromptOutcome,
    ) -> Result<NativeOwned<PromptOutcome>, JsonDecodeError> {
        let ending = match &value.ending {
            EndOfTurn::Finished => crate::model::native::session::finished(self.ledger)?,
            EndOfTurn::TokenLimit => crate::model::native::session::token_limit(self.ledger)?,
            EndOfTurn::RequestLimit => crate::model::native::session::request_limit(self.ledger)?,
            EndOfTurn::Refused => crate::model::native::session::refused(self.ledger)?,
            EndOfTurn::Cancelled => crate::model::native::session::cancelled(self.ledger)?,
            EndOfTurn::Other(value) => crate::model::native::session::other(self.text(value)?),
        };
        let tokens = match &value.tokens {
            Some(value) => NativeOwned::supplied(crate::model::native::session::totals(
                value.total,
                value.input,
                value.output,
                value.thought,
                value.cache_read,
                value.cache_write,
                self.ledger,
            )?),
            None => NativeOwned::absent(self.ledger)?,
        };
        crate::model::native::session::outcome(ending, tokens)
    }
}
