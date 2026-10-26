//! Typed reads of observations. Each marker names one observation capability and the data it
//! carries.

use nocterm_plugin_api::{
    HostError, Observed,
    capability::{
        Capability,
        clock::{ClockNow, Now},
        monitor::{Usage as UsageData, UsageNow},
        notifications::{Own, OwnNotices},
        transfers::{Counts as CountsData, CountsNow},
        workspace::{Kind, WorkspaceState},
    },
};
use thiserror::Error;

/// An observation capability, by marker: `cx.observe::<Clock>()`.
pub trait Observable {
    /// The capability id, as a manifest lists it.
    const ID: &'static str;
    /// The data the observation carries.
    type Value;

    /// The data of an observation, if it is this one.
    fn pick(observed: &Observed) -> Option<&Self::Value>;

    /// The observation that carries `value`, for a test host to put into a snapshot.
    fn wrap(value: Self::Value) -> Observed;
}

/// Why `Cx::observe` has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ObserveError {
    /// The manifest does not declare the capability.
    #[error("the manifest does not declare this capability")]
    Missing,
    /// The host refused or has no data, for example an optional capability the user did not grant.
    #[error("the host gave no data")]
    Host(#[source] HostError),
    /// The snapshot holds another kind of data under the id. A host never does this.
    #[error("the snapshot holds another kind of data")]
    Kind,
}

macro_rules! observable {
    ($(#[$doc:meta])* $marker:ident, $capability:ty, $value:ty, $variant:ident) => {
        $(#[$doc])*
        pub struct $marker;

        impl Observable for $marker {
            const ID: &'static str = <$capability as Capability>::ID;
            type Value = $value;

            fn pick(observed: &Observed) -> Option<&$value> {
                match observed {
                    Observed::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn wrap(value: $value) -> Observed {
                Observed::$variant(value)
            }
        }
    };
}

observable!(
    /// `clock.now`: the wall clock to the minute.
    Clock, Now, ClockNow, Clock
);
observable!(
    /// `workspace.kind`: the kind of the window's workspace and whether it is connected.
    Workspace, Kind, WorkspaceState, Workspace
);
observable!(
    /// `monitor.usage`: processor and memory use in percent.
    Usage, UsageNow, UsageData, Usage
);
observable!(
    /// `transfers.counts`: how many transfers run and how many failed.
    Counts, CountsNow, CountsData, Counts
);
observable!(
    /// `notifications.own`: the plugin's own live notices.
    Notices, Own, OwnNotices, Notices
);

/// The ids of every marker, which a test compares with the capability table.
#[cfg(test)]
pub(crate) const MARKER_IDS: [&str; 5] =
    [Clock::ID, Workspace::ID, Usage::ID, Counts::ID, Notices::ID];
