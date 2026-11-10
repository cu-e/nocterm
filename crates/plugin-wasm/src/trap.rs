//! What a wasmi error means to the host.

use nocterm_plugin_api::{Cause, ModuleError, RuntimeError};
use wasmi::{
    Error, TrapCode,
    errors::{ErrorKind, FuelError, InstantiationError, MemoryError, TableError},
};

/// Maps a wasmi error to the runtime error the host counts.
///
/// - Out of fuel is a missed deadline, a refused growth is the memory limit, other traps are
///   traps.
/// - Errors of the module itself (read, validation, translation, linking, instantiation,
///   limits) mean the module is refused; wasmi reports a few of them only when a function is
///   first translated, that is, during a call.
pub(crate) fn runtime_error(error: Error) -> RuntimeError {
    if let Some(code) = error.as_trap_code() {
        return match code {
            TrapCode::OutOfFuel => RuntimeError::Deadline,
            TrapCode::GrowthOperationLimited => RuntimeError::Memory,
            _ => RuntimeError::Trap,
        };
    }
    match error.kind() {
        ErrorKind::Memory(MemoryError::OutOfFuel { .. })
        | ErrorKind::Table(TableError::OutOfFuel { .. })
        | ErrorKind::Fuel(FuelError::OutOfFuel { .. }) => RuntimeError::Deadline,
        ErrorKind::Memory(_)
        | ErrorKind::Instantiation(InstantiationError::FailedToInstantiateMemory(_)) => {
            RuntimeError::Memory
        }
        ErrorKind::Linker(_)
        | ErrorKind::Table(_)
        | ErrorKind::Instantiation(_)
        | ErrorKind::Read(_)
        | ErrorKind::Wasm(_)
        | ErrorKind::Translation(_)
        | ErrorKind::UserLimits(_)
        | ErrorKind::ImplementationLimits(_)
        | ErrorKind::Ir(_) => RuntimeError::Module(ModuleError::Invalid(Cause::new(error))),
        _ => RuntimeError::Trap,
    }
}
