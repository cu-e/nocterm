use crate::sandbox::{Availability, SandboxProbe};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone)]
pub struct ScriptedSandboxProbe {
    availability: Availability,
    calls: Arc<AtomicUsize>,
}
impl ScriptedSandboxProbe {
    pub fn new(availability: Availability) -> Self {
        Self {
            availability,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl SandboxProbe for ScriptedSandboxProbe {
    fn availability(&self) -> Availability {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.availability.clone()
    }
}
