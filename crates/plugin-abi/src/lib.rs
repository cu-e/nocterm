//! The ABI between the host and a code plugin: export names, the call and reply wire types and
//! their postcard encoding. Shared by the wasm runtime adapter and the guest SDK.
//!
//! Owns: ABI_VERSION, export names, Call/Reply/Wire* and their conversions from plugin-api
//!       values, size bounds, canonical samples for the golden file.
//! Does not own: running modules (nocterm-plugin-wasm), package files (nocterm-plugin-packages),
//!       manifests and views (nocterm-plugin-api).
//! Ports: none. Settings: none. Globals: none.
//! Invariants: no_std + alloc; wire enums are append-only; any change to an existing sample's
//!       bytes bumps ABI_VERSION (xtask). Under the xtask isolation rules (I1-I5).
//!
//! A guest exports `memory`, `nocterm_abi_version`, `nocterm_alloc` and `nocterm_call`
//! ([`names`]) and imports nothing. The host writes an encoded [`Call`] into the buffer
//! `nocterm_alloc` returns, calls `nocterm_call(ptr, len)` and reads an encoded [`Reply`] at the
//! `(ptr, len)` pair that [`unpack`] takes out of the returned `i64`.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod codec;
pub mod names;
mod samples;
mod wire;

pub use codec::{AbiError, decode_call, decode_reply, encode_call, encode_reply};
pub use samples::{Sample, samples};
pub use wire::{Answer, Call, Reply, WireObserved, WireSetting, WireSnapshot};

/// The version of this ABI. A change to the bytes of any existing sample bumps it.
pub const ABI_VERSION: u32 = 1;
/// The largest encoded call the host writes into guest memory.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;
/// The largest encoded reply the host reads from guest memory. The host decodes a reply on the
/// calling thread, so the bound is also a bound on that work; the largest valid view (256 nodes
/// of text) is about 200 KiB.
pub const MAX_REPLY_BYTES: usize = 256 * 1024;
/// Most log lines a reply carries. A reply with more is malformed and counts as a fault.
pub const MAX_LOG_LINES: usize = 8;

/// Packs a guest pointer and a length into the `i64` bits `nocterm_call` returns.
pub const fn pack(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | len as u64
}

/// The pointer and length inside a value made by [`pack`].
pub const fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

#[cfg(test)]
mod tests;
