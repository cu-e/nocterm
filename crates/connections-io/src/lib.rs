//! Native adapters for connection country and flag observations.
//!
//! Owns: local DNS, bounded HTTP responses and private atomic flag cache files.
//! Does not own: profile rules, GPUI images, observation scheduling or persistence.
//! Ports used: none. Ports provided: PublicAddressResolver, CountryLocator,
//! FlagRepository. Settings section: none. Globals: none.
//! Invariants: country services receive public IPs only; bodies and cached flags
//! are bounded; failure to cache never discards an acquired flag.

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
