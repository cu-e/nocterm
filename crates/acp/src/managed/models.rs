//! Legacy model controls retain distinct prepaid cache and delivered components.
use super::{budget::SourceBudget, configuration_copy, copy, error::DecodeError};
use agent_client_protocol::managed::{Index, Source};
use nocterm_ai::model::{
    controls::{ConfigCategory, ConfigKind, ConfigOption, SelectChoices},
    json::deferred::JsonDecodeError,
    native::{NativeList, NativeOwned},
};
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
};
type Error = DecodeError<JsonDecodeError>;
struct SessionKey(Source<SourceBudget>);
impl Borrow<str> for SessionKey {
    fn borrow(&self) -> &str {
        self.0.text()
    }
}
impl Hash for SessionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.text().hash(state);
    }
}
impl PartialEq for SessionKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.text() == other.0.text()
    }
}
impl Eq for SessionKey {}
struct Cached {
    selector: Source<SourceBudget>,
    options: NativeOwned<Vec<ConfigOption>>,
}
pub(super) struct Models {
    entries: Index<SessionKey, Cached, SourceBudget>,
    budget: SourceBudget,
}
impl Models {
    pub(super) fn new(budget: SourceBudget) -> Self {
        Self {
            entries: Index::new(budget.clone()),
            budget,
        }
    }
    /// Copies both keys independently; no original RPC line is pinned by a cache.
    pub(super) fn install(
        &mut self,
        session: &str,
        selector: &str,
        options: &[ConfigOption],
    ) -> Result<(), Error> {
        let key = SessionKey(Source::copy(session, self.budget.clone())?);
        let value = Cached {
            selector: Source::copy(selector, self.budget.clone())?,
            options: configuration_copy::options(&self.budget, options, None)?,
        };
        self.entries
            .insert(key, value)
            .map_err(|(error, _, _)| DecodeError::Allocation(error))?;
        Ok(())
    }
    pub(super) fn remove(&mut self, session: &str) {
        self.entries.remove(session);
    }
    /// Returns a distinct paid selected ID only for the original offered selector.
    pub(super) fn selection(
        &self,
        session: &str,
        config: &str,
        selection: Option<&str>,
    ) -> Result<Option<NativeOwned<String>>, Error> {
        let Some(cached) = self.entries.get(session) else {
            return Ok(None);
        };
        if config != cached.selector.text() {
            return Ok(None);
        }
        let selection = selection.ok_or(DecodeError::Field("model selection value"))?;
        let valid = cached.options.get().iter().any(|option| {
            option.id.as_str() == config
                && match &option.kind {
                    ConfigKind::Select {
                        choices: SelectChoices::Flat(values),
                        ..
                    } => values.iter().any(|value| value.id.as_str() == selection),
                    _ => false,
                }
        });
        if !valid {
            return Err(DecodeError::Field("available model selection"));
        }
        Ok(Some(copy::text(&self.budget, selection)?))
    }
    /// Commit only after both actual cache and delivery copies are admitted.
    pub(super) fn selected(
        &mut self,
        session: &str,
        selection: Option<&str>,
    ) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
        let Some(cached) = self.entries.get(session) else {
            return Ok(NativeList::new(self.budget.ledger(), 0)?.finish());
        };
        let update = selection.map(|value| (cached.selector.text(), value));
        let stored = configuration_copy::options(&self.budget, cached.options.get(), update)?;
        let delivered = configuration_copy::options(&self.budget, stored.get(), None)?;
        if let Some(cached) = self.entries.get_mut(session) {
            cached.options = stored;
        }
        Ok(delivered)
    }
    pub(super) fn retain(
        &mut self,
        session: &str,
        options: NativeOwned<Vec<ConfigOption>>,
    ) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
        if options
            .get()
            .iter()
            .any(|value| value.category == Some(ConfigCategory::Model))
        {
            self.remove(session);
            return Ok(options);
        }
        let Some(cached) = self.entries.get(session) else {
            return Ok(options);
        };
        let extra = cached
            .options
            .get()
            .iter()
            .find(|value| value.id.as_str() == cached.selector.text());
        let mut result = NativeList::new(
            self.budget.ledger(),
            options.get().len() + usize::from(extra.is_some()),
        )?;
        for value in options.get() {
            result.push(configuration_copy::option(&self.budget, value, None)?)?;
        }
        if let Some(value) = extra {
            result.push(configuration_copy::option(&self.budget, value, None)?)?;
        }
        let delivered = result.finish();
        let stored = configuration_copy::options(&self.budget, delivered.get(), None)?;
        if let Some(cached) = self.entries.get_mut(session) {
            cached.options = stored;
        }
        Ok(delivered)
    }
}
