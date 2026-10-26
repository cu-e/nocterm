//! One installed code plugin: the guest, restarted fresh after any failure.

use std::sync::{Arc, Mutex, PoisonError};

use nocterm_plugin_abi::{Answer, Call, WireSnapshot, encode_call};
use nocterm_plugin_api::{
    Effects, Event, LocalId, Manifest, PluginInstance, ProtocolError, RuntimeError, Snapshot, View,
};
use wasmi::{Engine, Module};

use crate::{
    budget::LiveSlot,
    guest::Guest,
    log::{GuestLog, LogBudget},
};

/// The mutable part of an instance: the guest, when one is running, and what it has logged.
struct Running {
    guest: Option<Guest>,
    log: LogBudget,
}

pub(crate) struct WasmInstance {
    running: Mutex<Running>,
    module: Module,
    engine: Engine,
    manifest: Manifest,
    _slot: LiveSlot,
}

impl WasmInstance {
    /// Starts the first guest, so a module the ABI refuses is refused at instantiation.
    pub(crate) fn start(
        engine: Engine,
        module: Module,
        manifest: Manifest,
        sink: Arc<dyn GuestLog>,
        slot: LiveSlot,
    ) -> Result<Self, RuntimeError> {
        let guest = Guest::start(&engine, &module)?;
        let plugin = manifest.plugin().id().clone();
        Ok(Self {
            running: Mutex::new(Running {
                guest: Some(guest),
                log: LogBudget::new(plugin, sink),
            }),
            module,
            engine,
            manifest,
            _slot: slot,
        })
    }

    /// Sends `call` to the guest and takes the part of the answer `pick` wants.
    ///
    /// A guest that is missing (it failed before) is started again from the cached module, so
    /// its state is fresh. A guest that fails, or whose answer `pick` refuses, is dropped.
    fn run<T>(
        &self,
        local: &LocalId,
        call: &Call,
        pick: fn(Answer) -> Option<T>,
    ) -> Result<T, RuntimeError> {
        if !self.manifest.has_contribution(local) {
            return Err(RuntimeError::UnknownContribution(local.clone()));
        }
        // Encoded before the guest is taken: a call the host cannot encode is no fault of the
        // plugin and leaves its state alone.
        let input = encode_call(call).map_err(|_| RuntimeError::InputTooLarge)?;
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if self.running.is_poisoned() {
            // A panic left the guest in an unknown state.
            running.guest = None;
            self.running.clear_poison();
        }
        let mut guest = match running.guest.take() {
            Some(guest) => guest,
            None => Guest::start(&self.engine, &self.module)?,
        };
        let reply = guest.call(&input)?;
        let Some(value) = pick(reply.answer) else {
            return Err(ProtocolError::WrongAnswer.into());
        };
        running.log.record(&reply.log);
        running.guest = Some(guest);
        Ok(value)
    }
}

impl PluginInstance for WasmInstance {
    fn render(&self, local: &LocalId, snapshot: &Snapshot) -> Result<View, RuntimeError> {
        let call = Call::Render {
            local: local.clone(),
            snapshot: WireSnapshot::from(snapshot),
        };
        self.run(local, &call, |answer| match answer {
            Answer::View(view) => Some(view),
            Answer::Effects(_) => None,
        })
    }

    fn on_event(
        &self,
        local: &LocalId,
        event: Event,
        snapshot: &Snapshot,
    ) -> Result<Effects, RuntimeError> {
        let call = Call::Event {
            local: local.clone(),
            event,
            snapshot: WireSnapshot::from(snapshot),
        };
        self.run(local, &call, |answer| match answer {
            Answer::Effects(effects) => Some(effects),
            Answer::View(_) => None,
        })
    }
}
