//! Where an agent's work runs.

/// The machine an agent works on.
///
/// A personal agent that works while the person is offline lives on a pool of the hub. One
/// that works only while a device of the person is online runs on that person's own runner:
/// no sealed copy of the key leaves the device, and the device leases the provider credential
/// and renews the lease in the background for as long as it is online.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Placement {
    /// A host of a pool of the hub runs the agent; no device needs to be online.
    Pool,
    /// The person's own runner on this device runs the agent.
    OwnRunner,
}
