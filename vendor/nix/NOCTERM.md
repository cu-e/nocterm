# Nocterm native caller-buffer addition

Base: published nix 0.28.0, upstream 21ab06ef23de214174ddb039be5b5f08750d19e6, MIT.

Nocterm adds fs-feature `unistd::getcwd_into` with initialized caller-owned
storage. It calls the original libc getcwd, preserving native errors and
allowing the caller to prepay Rust buffer growth. Existing getcwd is unchanged.
libc-internal allocation is outside its Rust-buffer contract.

Behavior cases extend the existing unistd tests and reuse its current-directory
restore/lock owner. Windows is not implemented by this Unix-only fork.

The existing socket address test uses an explicit borrow of the raw-pointer
field before slicing, preserving its assertion on current Rust compilers.

Fs-feature, Unix-only `getenv_into`, `environment_into` and `passwd_home_into`
copy native environment values, the complete environment and the password home
into caller-owned storage; `passwd_home_into` keeps the platform exclusions of
the surrounding passwd API. `environment_into` keeps raw entry order, duplicates, empty
values and each NUL, or reports the exact byte and entry requirement without
writing. None caches a native pointer, takes std's private environment lock or
relaxes the safety obligations of environment mutators. `realpath_owned` keeps
libc's original canonical-path allocation alive behind an owner that frees it with
libc `free`, so callers admit their Rust copy before making it.

Build hygiene: `build.rs` declares its cfg aliases and upstream's CI-only
`fbsd14` for rustc's cfg checking. The Fuchsia `SigSet` cfg spelling is corrected
to `target_os = "fuchsia"`, matching later upstream releases. The crate allows
`missing_abi`, `mismatched_lifetime_syntaxes` and
`unpredictable_function_pointer_comparisons`, which warn by default only on
compilers newer than this release, about unchanged upstream code.
