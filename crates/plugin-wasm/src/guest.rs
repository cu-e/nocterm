//! One running module: its store, its memory and the two exports the host calls.

use nocterm_plugin_abi::{
    ABI_VERSION, CALL_FUEL, MAX_LOG_LINES, MAX_REPLY_BYTES, Reply, START_FUEL, decode_reply,
    names::{EXPORT_ALLOC, EXPORT_CALL, EXPORT_MEMORY, EXPORT_VERSION},
    unpack,
};
use nocterm_plugin_api::{Cause, ModuleError, ProtocolError, RuntimeError};
use wasmi::{Engine, ExternType, Linker, Memory, Module, Store, StoreLimits, TypedFunc, ValType};

use crate::{engine::store_limits, trap::runtime_error};

/// A started instance of a module that passed the ABI checks.
pub(crate) struct Guest {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    call: TypedFunc<(i32, i32), i64>,
}

impl Guest {
    /// Instantiates `module` and checks it against the ABI.
    ///
    /// The module must import nothing and export `memory` and the three functions with the ABI
    /// types; its start function and `nocterm_abi_version` run under [`START_FUEL`].
    pub(crate) fn start(engine: &Engine, module: &Module) -> Result<Self, RuntimeError> {
        if module.imports().next().is_some() {
            return Err(ModuleError::Imports.into());
        }
        check_exports(module)?;
        let mut store = Store::new(engine, store_limits());
        store.limiter(|limits| limits);
        store.set_fuel(START_FUEL).map_err(runtime_error)?;
        let instance = Linker::<StoreLimits>::new(engine)
            .instantiate_and_start(&mut store, module)
            .map_err(runtime_error)?;
        let exports = |_| RuntimeError::from(ModuleError::Exports);
        let memory = instance
            .get_memory(&store, EXPORT_MEMORY)
            .ok_or(ModuleError::Exports)?;
        let version = instance
            .get_typed_func::<(), i32>(&store, EXPORT_VERSION)
            .map_err(exports)?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, EXPORT_ALLOC)
            .map_err(exports)?;
        let call = instance
            .get_typed_func::<(i32, i32), i64>(&store, EXPORT_CALL)
            .map_err(exports)?;
        let found = version.call(&mut store, ()).map_err(runtime_error)?;
        if u32::try_from(found) != Ok(ABI_VERSION) {
            return Err(ModuleError::AbiVersion { found }.into());
        }
        Ok(Self {
            store,
            memory,
            alloc,
            call,
        })
    }

    /// Runs one call under [`CALL_FUEL`]: writes the encoded call `input` into a buffer the
    /// guest allocated, runs `nocterm_call` and reads the reply it points at.
    ///
    /// Every pointer the guest returns is checked against the memory and the reply bound before
    /// it is read, and a reply with more than [`MAX_LOG_LINES`] log lines is refused. The caller
    /// drops the guest on any error.
    pub(crate) fn call(&mut self, input: &[u8]) -> Result<Reply, RuntimeError> {
        let len = i32::try_from(input.len()).map_err(|_| RuntimeError::InputTooLarge)?;
        self.store.set_fuel(CALL_FUEL).map_err(runtime_error)?;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(runtime_error)?;
        self.memory
            .write(&mut self.store, ptr as u32 as usize, input)
            .map_err(|_| ProtocolError::OutsideMemory)?;
        let packed = self
            .call
            .call(&mut self.store, (ptr, len))
            .map_err(runtime_error)?;
        let (reply_ptr, reply_len) = unpack(packed as u64);
        let reply_len = reply_len as usize;
        if reply_len > MAX_REPLY_BYTES {
            return Err(ProtocolError::ReplyTooLarge {
                max: MAX_REPLY_BYTES,
            }
            .into());
        }
        let mut bytes = vec![0; reply_len];
        self.memory
            .read(&self.store, reply_ptr as usize, &mut bytes)
            .map_err(|_| ProtocolError::OutsideMemory)?;
        let reply =
            decode_reply(&bytes).map_err(|error| ProtocolError::Undecodable(Cause::new(error)))?;
        if reply.log.len() > MAX_LOG_LINES {
            return Err(ProtocolError::TooManyLogLines { max: MAX_LOG_LINES }.into());
        }
        Ok(reply)
    }
}

/// The exports of `module` against the ABI, before anything runs.
fn check_exports(module: &Module) -> Result<(), RuntimeError> {
    let func = |name: &str, params: &[ValType], results: &[ValType]| {
        matches!(
            module.get_export(name),
            Some(ExternType::Func(ty)) if ty.params() == params && ty.results() == results
        )
    };
    let memory = matches!(
        module.get_export(EXPORT_MEMORY),
        Some(ExternType::Memory(ty)) if !ty.is_64()
    );
    let ok = memory
        && func(EXPORT_VERSION, &[], &[ValType::I32])
        && func(EXPORT_ALLOC, &[ValType::I32], &[ValType::I32])
        && func(EXPORT_CALL, &[ValType::I32, ValType::I32], &[ValType::I64]);
    if ok {
        Ok(())
    } else {
        Err(ModuleError::Exports.into())
    }
}
