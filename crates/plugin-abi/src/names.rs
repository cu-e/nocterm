//! The exports of a guest module. A module has no imports.

/// The linear memory the host reads and writes.
pub const EXPORT_MEMORY: &str = "memory";
/// `() -> i32`: the ABI version the guest was built for.
pub const EXPORT_VERSION: &str = "nocterm_abi_version";
/// `(i32) -> i32`: a buffer of the given length for the next call; the pointer stays valid
/// until the next `nocterm_alloc` or `nocterm_call`.
pub const EXPORT_ALLOC: &str = "nocterm_alloc";
/// `(i32, i32) -> i64`: handles the call at `(ptr, len)` and returns the packed `(ptr, len)` of
/// the reply.
pub const EXPORT_CALL: &str = "nocterm_call";

/// Every export with its signature, in the order the documentation lists them.
pub const EXPORTS: [(&str, &str); 4] = [
    (EXPORT_MEMORY, "memory"),
    (EXPORT_VERSION, "() -> i32"),
    (EXPORT_ALLOC, "(i32) -> i32"),
    (EXPORT_CALL, "(i32, i32) -> i64"),
];
