//! The `export!` macro: the only place the guest side uses `unsafe`.

/// Exports a [`Plugin`](crate::Plugin) as a code plugin module.
///
/// `$plugin` is the plugin type; `$manifest` is the path of its `plugin.toml`, as
/// `include_bytes!` reads it (an expression, so `concat!(env!("CARGO_MANIFEST_DIR"), ..)` works).
/// The expansion, on `wasm32` only:
///
/// - puts the manifest into the custom section `nocterm.manifest`, which is what makes the
///   module one package;
/// - exports `nocterm_abi_version`, `nocterm_alloc` and `nocterm_call`, which hand every call to
///   [`dispatch`](crate::__private::dispatch);
/// - keeps the plugin value, the input buffer and the reply buffer in one static slot.
///
/// On other targets it only checks that `$plugin` is a plugin and that the manifest exists, so an
/// author's crate builds and tests natively.
///
/// The slot is an `UnsafeCell` with `unsafe impl Sync`. That is sound because a
/// `wasm32-unknown-unknown` module has no threads, the host calls the exports one at a time, and
/// the module imports nothing that could call back into it. A plugin that panics traps the
/// module; the host drops the instance and starts a fresh one on the next call.
///
/// The author's crate must not `forbid(unsafe_code)`: a macro cannot lower a `forbid`.
///
/// ```ignore
/// nocterm_plugin_sdk::export!(MyPlugin, concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.toml"));
/// ```
#[macro_export]
macro_rules! export {
    ($plugin:ty, $manifest:expr $(,)?) => {
        const _: () = {
            const fn __assert_plugin<P: $crate::Plugin>() {}
            __assert_plugin::<$plugin>();

            #[cfg(not(target_arch = "wasm32"))]
            const _: &[u8] = ::core::include_bytes!($manifest);

            #[cfg(target_arch = "wasm32")]
            #[used]
            #[unsafe(link_section = "nocterm.manifest")]
            static __NOCTERM_MANIFEST: [u8; ::core::include_bytes!($manifest).len()] =
                *::core::include_bytes!($manifest);

            #[cfg(target_arch = "wasm32")]
            struct __Slot {
                plugin: ::core::option::Option<$plugin>,
                input: $crate::__private::Vec<u8>,
                reply: $crate::__private::Vec<u8>,
            }

            #[cfg(target_arch = "wasm32")]
            struct __Cell(::core::cell::UnsafeCell<__Slot>);

            // The premise of the `Sync` impl below, stated by the build: a module with shared
            // memory could run exports on several threads.
            #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
            ::core::compile_error!(
                "nocterm plugins are single-threaded: build without the `atomics` target feature"
            );

            // SAFETY: a wasm32-unknown-unknown module has one thread, and the host never calls
            // an export while another is running.
            #[cfg(target_arch = "wasm32")]
            unsafe impl ::core::marker::Sync for __Cell {}

            #[cfg(target_arch = "wasm32")]
            static __SLOT: __Cell = __Cell(::core::cell::UnsafeCell::new(__Slot {
                plugin: ::core::option::Option::None,
                input: $crate::__private::Vec::new(),
                reply: $crate::__private::Vec::new(),
            }));

            /// # Safety
            ///
            /// The caller must be an export, which is never re-entered.
            #[cfg(target_arch = "wasm32")]
            unsafe fn __slot() -> &'static mut __Slot {
                // SAFETY: see `Sync for __Cell`; the exports are the only callers and none runs
                // while another holds the reference.
                unsafe { &mut *__SLOT.0.get() }
            }

            #[cfg(target_arch = "wasm32")]
            #[unsafe(no_mangle)]
            pub extern "C" fn nocterm_abi_version() -> i32 {
                $crate::__private::ABI_VERSION as i32
            }

            #[cfg(target_arch = "wasm32")]
            #[unsafe(no_mangle)]
            pub extern "C" fn nocterm_alloc(len: i32) -> i32 {
                let len = len as u32 as usize;
                ::core::assert!(
                    len <= $crate::__private::MAX_INPUT_BYTES,
                    "the call is larger than the ABI allows"
                );
                // SAFETY: an export, see `__slot`.
                let slot = unsafe { __slot() };
                slot.input.clear();
                slot.input.resize(len, 0);
                slot.input.as_ptr() as u32 as i32
            }

            #[cfg(target_arch = "wasm32")]
            #[unsafe(no_mangle)]
            pub extern "C" fn nocterm_call(ptr: i32, len: i32) -> i64 {
                // SAFETY: an export, see `__slot`.
                let slot = unsafe { __slot() };
                let len = len as u32 as usize;
                ::core::assert!(
                    ptr as u32 as usize == slot.input.as_ptr() as usize && len <= slot.input.len(),
                    "the call is not the buffer nocterm_alloc returned"
                );
                let plugin = slot
                    .plugin
                    .get_or_insert_with(<$plugin as ::core::default::Default>::default);
                match $crate::__private::dispatch(plugin, &slot.input[..len]) {
                    ::core::result::Result::Ok(reply) => {
                        slot.reply = reply;
                        $crate::__private::pack(slot.reply.as_ptr() as u32, slot.reply.len() as u32)
                            as i64
                    }
                    ::core::result::Result::Err(error) => {
                        ::core::panic!("the call broke the ABI: {error}")
                    }
                }
            }
        };
    };
}
