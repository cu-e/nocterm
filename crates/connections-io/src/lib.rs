//! Native connection files, environment/entropy/clock and network observations.
//!
//! Owns: three-document repositories, native defaults/UUID/clock, DNS, bounded remote OS reads and HTTP/flag cache.
//! Does not own: profile rules, GPUI images, observation scheduling or persistence workers.
//! Ports used: foundation persistence. Ports provided: ConnectionRepository, ProfileIds,
//! ConnectionClock, PublicAddressResolver, CountryLocator,
//! FlagRepository, RemoteOsDetector. Settings section: none. Globals: none.
//! Invariants: country services receive public IPs only; bodies and cached flags
//! are bounded; failure to cache never discards an acquired flag. Blocking document
//! acquisition and writes run off foreground; profiles retain configuration publication semantics.

mod os_probe;
pub use os_probe::NativeOsDetector;
mod native_values;
mod repository;
pub use native_values::NativeValues;
pub use repository::FileConnectionRepository;
mod flags;
mod http;
mod resolver;

use nocterm_connections_model::observation::{
    CountryLocator, FlagRepository, PublicAddressResolver,
};
use std::{path::PathBuf, sync::Arc};

pub use resolver::NativeResolver;

/// A native capability bundle constructed only by application composition.
pub struct NativeObservations {
    resolver: Arc<dyn PublicAddressResolver>,
    locator: Arc<dyn CountryLocator>,
    flags: Arc<dyn FlagRepository>,
}

impl NativeObservations {
    pub fn new(flags_dir: PathBuf) -> Self {
        Self {
            resolver: Arc::new(NativeResolver),
            locator: Arc::new(http::HttpCountry::default()),
            flags: Arc::new(flags::CachedFlags::new(flags_dir)),
        }
    }

    pub fn resolver(&self) -> Arc<dyn PublicAddressResolver> {
        self.resolver.clone()
    }
    pub fn locator(&self) -> Arc<dyn CountryLocator> {
        self.locator.clone()
    }
    pub fn flags(&self) -> Arc<dyn FlagRepository> {
        self.flags.clone()
    }
}

#[cfg(test)]
mod test_support;
