# Nocterm native caller-buffer addition

Base: published windows-sys0.61.2, upstream
32c3144490c016fe496a0aed769bce60987a2e9d, crates/libs/sys,
MIT OR Apache-2.0.

Nocterm adds the safe `Win32::System::Environment::caller_buffer` facade when
Win32_System_Environment and Win32_Foundation are enabled. It borrows initialized
caller UTF-16 storage, invokes the existing GetCurrentDirectoryW declaration,
and immediately preserves native GetLastError on failure. It adds no std/alloc
dependency, native text conversion, path limit, normalization or owned buffer.
Required capacity and successful original-buffer slices retain native meanings.

Windows native tests compare the original borrowed buffer and std current_dir
and exercise native capacity reporting. Cross compilation alone does not prove
native execution, allocator behavior or platform fault handling.
