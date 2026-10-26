//! A native host for plugin tests. It drives the same [`dispatch`](crate::__private::dispatch)
//! the exported module runs, with the real encoding, and applies the host's rules to what comes
//! back: a view is validated, effects are checked and answered `NotGranted`, `Denied` or `Ok`,
//! storage effects change the stored data, and the outcomes arrive in the next snapshot.
//!
//! ```
//! use nocterm_plugin_sdk::{Cx, Plugin, ViewBuilder, api::View, testing::Host};
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! impl Plugin for Hello {
//!     fn render(&mut self, _cx: &mut Cx<'_>) -> View {
//!         let mut view = ViewBuilder::new();
//!         view.text(None, "hello");
//!         view.build()
//!     }
//! }
//!
//! let manifest = r#"
//!     api = "0.1"
//!     runtime = "wasm"
//!     publisher = "example"
//!     id = "hello"
//!     name = "Hello"
//!     [[contribution]]
//!     local = "face"
//!     kind = "widget"
//!     title = "Hello"
//! "#;
//! let mut host = Host::new(manifest).unwrap();
//! let view = host.render(&mut Hello, "face").unwrap();
//! assert_eq!(view.nodes.len(), 1);
//! ```

use std::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use nocterm_plugin_abi::{
    AbiError, Answer, Call, MAX_LOG_LINE_CHARS, MAX_LOG_LINES, WireSnapshot, decode_reply,
    encode_call,
};
use nocterm_plugin_api::{
    API_VERSION, ContributionContext, Effect, EffectOutcome, EffectRequest, Effects, EffectsError,
    Event, HostContext, HostError, Kind, LimitError, LocalId, Manifest, ManifestDto, ManifestError,
    Observed, OptionalCapability, ProtocolError, Runtime, Snapshot, View, ViewError, Zone,
    capability::capability, capability::storage::MAX_TOTAL, sanitize_display,
};
use nocterm_plugin_model::{ContributionId, ContributionKind, SettingValue, SettingValues};
use thiserror::Error;

use crate::{__private::dispatch, Observable, Plugin};

/// Why a test call did not produce what the plugin meant.
#[derive(Debug, Error)]
pub enum TestError {
    #[error("the manifest is not valid TOML for a package")]
    Toml(#[source] toml::de::Error),
    #[error("the manifest is refused")]
    Manifest(#[source] ManifestError),
    #[error("the manifest is not a code package: set `runtime = \"wasm\"`")]
    NotCode,
    #[error("`{0}` is not a valid contribution name")]
    Local(String, #[source] LimitError),
    #[error("the manifest declares no contribution `{0}`")]
    UnknownContribution(LocalId),
    #[error("the call or the reply broke the ABI")]
    Abi(#[source] AbiError),
    #[error("the host would refuse the view")]
    View(#[source] ViewError),
    #[error("the host would drop the effects")]
    Effects(#[source] EffectsError),
    #[error("the host would refuse the reply")]
    Reply(#[source] ProtocolError),
    #[error("the plugin answered with the wrong kind of reply")]
    Answer,
    #[error("`{0}` is a required capability of the manifest; only an optional one can be denied")]
    Required(String),
}

/// A host for one plugin package.
pub struct Host {
    manifest: Manifest,
    observations: BTreeMap<String, Result<Observed, HostError>>,
    settings: SettingValues,
    storage: BTreeMap<String, String>,
    outcomes: Vec<EffectOutcome>,
    denied: BTreeSet<String>,
    log: Vec<String>,
}

impl Host {
    /// A host for the package whose `plugin.toml` is `manifest_toml`. Settings start at the
    /// defaults of the manifest's schema.
    pub fn new(manifest_toml: &str) -> Result<Self, TestError> {
        let dto: ManifestDto = toml::from_str(manifest_toml).map_err(TestError::Toml)?;
        let manifest = Manifest::validate(&dto, &HostContext::new(API_VERSION, &[]))
            .map_err(TestError::Manifest)?;
        if manifest.runtime() != Runtime::Wasm {
            return Err(TestError::NotCode);
        }
        Ok(Self {
            settings: manifest.plugin().settings().defaults(),
            manifest,
            observations: BTreeMap::new(),
            storage: BTreeMap::new(),
            outcomes: Vec::new(),
            denied: BTreeSet::new(),
            log: Vec::new(),
        })
    }

    /// Sets what an observation carries from now on. The snapshot holds it only if the manifest
    /// declares the capability.
    pub fn observe<T: Observable>(&mut self, value: T::Value) -> &mut Self {
        self.observations
            .insert(String::from(T::ID), Ok(T::wrap(value)));
        self
    }

    /// Makes an observation fail with `error`, as a provider without data does.
    pub fn observe_error<T: Observable>(&mut self, error: HostError) -> &mut Self {
        self.observations.insert(String::from(T::ID), Err(error));
        self
    }

    /// Sets the value of a setting.
    pub fn set(&mut self, key: &str, value: SettingValue) -> &mut Self {
        self.settings.insert(key, value);
        self
    }

    /// Stores a value before the plugin runs, as an earlier session left it.
    pub fn store(&mut self, key: &str, value: &str) -> &mut Self {
        self.storage.insert(String::from(key), String::from(value));
        self
    }

    /// Withholds an optional capability, as a user does who declines it: its effects get
    /// `Denied` and its observation is unavailable. The host never withholds a required one (it
    /// does not install the plugin without it), so this refuses it with
    /// [`TestError::Required`]; to test a required observation that fails, use
    /// [`observe_error`](Self::observe_error). A capability the manifest does not declare stays
    /// ungranted whether denied or not.
    pub fn deny(&mut self, capability: &str) -> Result<&mut Self, TestError> {
        if self
            .manifest
            .required()
            .iter()
            .any(|entry| entry.id == capability)
        {
            return Err(TestError::Required(String::from(capability)));
        }
        self.denied.insert(String::from(capability));
        Ok(self)
    }

    /// The plugin's stored data.
    pub fn storage(&self) -> &BTreeMap<String, String> {
        &self.storage
    }

    /// The log lines the host accepted, sanitized, over all calls.
    pub fn log(&self) -> &[String] {
        &self.log
    }

    /// Renders the contribution `local`; the host's validation applies to the view.
    pub fn render<P: Plugin>(&mut self, plugin: &mut P, local: &str) -> Result<View, TestError> {
        let local = self.contribution(local)?;
        let snapshot = self.snapshot(&local);
        let call = Call::Render {
            local,
            snapshot: WireSnapshot::from(&snapshot),
        };
        match self.call(plugin, &call)? {
            Answer::View(view) => view.into_validated().map_err(TestError::View),
            Answer::Effects(_) => Err(TestError::Answer),
        }
    }

    /// Sends `event` to the contribution `local` and applies the effects it answers with.
    pub fn event<P: Plugin>(
        &mut self,
        plugin: &mut P,
        local: &str,
        event: Event,
    ) -> Result<Effects, TestError> {
        let local = self.contribution(local)?;
        let snapshot = self.snapshot(&local);
        let call = Call::Event {
            local,
            event,
            snapshot: WireSnapshot::from(&snapshot),
        };
        let Answer::Effects(effects) = self.call(plugin, &call)? else {
            return Err(TestError::Answer);
        };
        effects.check().map_err(TestError::Effects)?;
        for request in &effects.0 {
            let result = self.settle(request);
            self.outcomes.push(EffectOutcome {
                key: request.key.clone(),
                result,
            });
        }
        Ok(effects)
    }

    fn contribution(&self, local: &str) -> Result<LocalId, TestError> {
        let parsed = LocalId::parse(local).map_err(|e| TestError::Local(String::from(local), e))?;
        if self.manifest.has_contribution(&parsed) {
            Ok(parsed)
        } else {
            Err(TestError::UnknownContribution(parsed))
        }
    }

    fn call<P: Plugin>(&mut self, plugin: &mut P, call: &Call) -> Result<Answer, TestError> {
        let input = encode_call(call).map_err(TestError::Abi)?;
        let output = dispatch(plugin, &input).map_err(TestError::Abi)?;
        let reply = decode_reply(&output).map_err(TestError::Abi)?;
        let lines = accepted_log(&reply.log).map_err(TestError::Reply)?;
        self.log.extend(lines);
        Ok(reply.answer)
    }

    fn snapshot(&mut self, local: &LocalId) -> Snapshot {
        let mut snapshot = Snapshot::new(self.context(local));
        for id in self.declared() {
            let Some(entry) = capability(id) else {
                continue;
            };
            if entry.family != nocterm_plugin_api::Family::Observe {
                continue;
            }
            let result = if self.denied.contains(id) {
                Err(HostError::Unavailable)
            } else {
                self.observations
                    .get(id)
                    .cloned()
                    .unwrap_or(Err(HostError::Unavailable))
            };
            snapshot.observations.insert(String::from(id), result);
        }
        snapshot.settings = self.settings.clone();
        snapshot.storage = self.storage.clone();
        snapshot.outcomes = core::mem::take(&mut self.outcomes);
        snapshot
    }

    fn context(&self, local: &LocalId) -> ContributionContext {
        let plugin = self.manifest.plugin();
        let found = ContributionId::in_plugin(plugin.id(), local.as_str())
            .ok()
            .and_then(|id| plugin.contributions().iter().find(|c| *c.id() == id));
        ContributionContext {
            kind: match found.map(|c| c.kind()) {
                Some(ContributionKind::Panel) => Kind::Panel,
                _ => Kind::Widget,
            },
            zone: match found.map(|c| c.placement().zone) {
                Some(nocterm_plugin_model::Zone::Left) => Zone::Left,
                Some(nocterm_plugin_model::Zone::Center) => Zone::Center,
                _ => Zone::Right,
            },
        }
    }

    fn declared(&self) -> impl Iterator<Item = &str> {
        self.manifest
            .required()
            .iter()
            .map(|entry| entry.id)
            .chain(self.manifest.optional().iter().map(OptionalCapability::id))
    }

    fn settle(&mut self, request: &EffectRequest) -> Result<(), HostError> {
        let effect = &request.effect;
        let id = effect.capability();
        if !self.declared().any(|declared| declared == id) {
            return Err(HostError::NotGranted);
        }
        if self.denied.contains(id) {
            return Err(HostError::Denied);
        }
        if effect
            .local()
            .is_some_and(|local| !self.manifest.has_contribution(local))
        {
            return Err(HostError::Unavailable);
        }
        match effect {
            Effect::StorageSet(set) => {
                let mut next = self.storage.clone();
                next.insert(set.key.clone(), set.value.clone());
                if total(&next) > MAX_TOTAL {
                    return Err(HostError::Denied);
                }
                self.storage = next;
            }
            Effect::StorageRemove(remove) => {
                self.storage.remove(&remove.key);
            }
            _ => {}
        }
        Ok(())
    }
}

/// The log lines of a reply as the host keeps them: sanitized and cut. A reply with more than
/// [`MAX_LOG_LINES`] is refused whole, as the host refuses it.
pub(crate) fn accepted_log(log: &[String]) -> Result<Vec<String>, ProtocolError> {
    if log.len() > MAX_LOG_LINES {
        return Err(ProtocolError::TooManyLogLines { max: MAX_LOG_LINES });
    }
    Ok(log
        .iter()
        .map(|line| sanitize_display(line, MAX_LOG_LINE_CHARS))
        .collect())
}

fn total(storage: &BTreeMap<String, String>) -> usize {
    storage.iter().map(|(k, v)| k.len() + v.len()).sum()
}
