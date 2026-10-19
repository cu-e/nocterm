//! Nocterm addition: native environment observations in caller-owned storage.
use std::ffi::CStr;

/// Native presence and exact caller-storage requirements, without UTF-8 conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentObservation {
    /// The variable is not set.
    Missing,
    /// The value was copied; its length in bytes, without a NUL.
    Copied(usize),
    /// The output is too small; the value's length in bytes. Nothing was written.
    Required(usize),
}

/// Copy a current native environment value without allocating a Rust backing.
///
/// Empty values are `Copied(0)`, distinct from a missing name. `Required` performs
/// no partial copy. No environment pointer, callback or guard escapes this call.
///
/// As with other native environment readers, concurrent environment mutation
/// must obey its own safety requirements. In particular, Rust's Unix
/// `std::env::set_var`/`remove_var` require excluding concurrent reads outside
/// `std::env`; this function does not relax that process-wide requirement or
/// pretend to acquire the standard library's private environment lock.
pub fn getenv_into(name: &CStr, output: &mut [u8]) -> EnvironmentObservation {
    // SAFETY: name is terminated and immutable. The returned pointer is consumed
    // only in this call; environment mutators must exclude concurrent native
    // reads, just as for libc readers reached through execvp or DNS.
    let pointer = unsafe { libc::getenv(name.as_ptr()) };
    if pointer.is_null() {
        return EnvironmentObservation::Missing;
    }
    // SAFETY: getenv returns a terminated value while no environment mutation
    // invalidates it. Nothing borrowed from it is returned or passed to a callback.
    let value = unsafe { CStr::from_ptr(pointer) }.to_bytes();
    if output.len() < value.len() {
        return EnvironmentObservation::Required(value.len());
    }
    output[..value.len()].copy_from_slice(value);
    EnvironmentObservation::Copied(value.len())
}
