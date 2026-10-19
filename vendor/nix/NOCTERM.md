# Nocterm native caller-buffer addition

Base: published nix0.28.0, upstream21ab06ef23de214174ddb039be5b5f08750d19e6, MIT.

Nocterm adds fs-feature `unistd::getcwd_into` with initialized caller-owned
storage. It calls the original libc getcwd, preserving native errors and
allowing the caller to prepay Rust buffer growth. Existing getcwd is unchanged.
libc-internal allocation is outside its Rust-buffer contract.

Behavior cases extend the existing unistd tests and reuse its current-directory
restore/lock owner. Windows is not implemented by this Unix-only fork.

The existing socket address test uses an explicit borrow of the raw-pointer
field before slicing, preserving its assertion on current Rust compilers.
