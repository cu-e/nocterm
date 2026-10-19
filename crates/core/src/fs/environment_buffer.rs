//! Native environment copies own their admitted Rust storage before observation.
use super::PathAdmissionError;
use std::{
    ffi::{CStr, OsStr},
    os::unix::ffi::OsStrExt,
};

pub enum EnvironmentObservation<'a> {
    Missing,
    Native(&'a OsStr),
    Required(usize),
}
pub struct EnvironmentBuffer<Credit> {
    storage: Vec<u8>,
    _credit: Credit,
}
impl<Credit> EnvironmentBuffer<Credit> {
    /// Prepay exactly capacity bytes before constructing this native output owner.
    pub fn new(capacity: usize, credit: Credit) -> Result<Self, PathAdmissionError> {
        let mut storage = Vec::new();
        storage.try_reserve_exact(capacity)?;
        if storage.capacity() != capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        storage.resize(capacity, 0);
        Ok(Self {
            storage,
            _credit: credit,
        })
    }
    pub fn query(&mut self, name: &CStr) -> EnvironmentObservation<'_> {
        use nix::unistd::EnvironmentObservation as Native;
        match nix::unistd::getenv_into(name, &mut self.storage) {
            Native::Missing => EnvironmentObservation::Missing,
            Native::Required(capacity) => EnvironmentObservation::Required(capacity),
            Native::Copied(length) => {
                EnvironmentObservation::Native(OsStr::from_bytes(&self.storage[..length]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_environment_observation_grows_without_changing_native_bytes() {
        let expected = std::env::var_os("TMPDIR");
        let mut buffer = EnvironmentBuffer::new(0, ()).unwrap();
        loop {
            match buffer.query(c"TMPDIR") {
                EnvironmentObservation::Required(capacity) => {
                    buffer = EnvironmentBuffer::new(capacity, ()).unwrap();
                }
                EnvironmentObservation::Missing => {
                    assert!(expected.is_none());
                    break;
                }
                EnvironmentObservation::Native(value) => {
                    assert_eq!(Some(value), expected.as_deref());
                    break;
                }
            }
        }
    }
}
