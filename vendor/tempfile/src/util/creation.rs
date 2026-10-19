//! One original creation/retry loop, parameterized only at allocation owners.
use crate::anonymous::{AnonymousOperations, Name};
use std::{ffi::OsStr, path::Path};

fn tmpname<O: AnonymousOperations>(
    operations: &mut O,
    rng: &mut fastrand::Rng,
    prefix: &OsStr,
    suffix: &OsStr,
    rand_len: usize,
) -> Result<Name<O::Credit>, O::Error> {
    let capacity = prefix
        .len()
        .saturating_add(suffix.len())
        .saturating_add(rand_len);
    let mut name = operations.name_buffer(capacity)?;
    name.push(prefix);
    let mut char_buf = [0; 4];
    for c in std::iter::repeat_with(|| rng.alphanumeric()).take(rand_len) {
        name.push(OsStr::new(c.encode_utf8(&mut char_buf)));
    }
    name.push(suffix);
    Ok(name)
}

pub(crate) fn create<O: AnonymousOperations, R>(
    operations: &mut O,
    base: &Path,
    prefix: &OsStr,
    suffix: &OsStr,
    random_len: usize,
    mut construct: impl FnMut(O::Path, &mut O) -> Result<R, O::Error>,
) -> Result<R, O::Error> {
    // The original helper observes cwd only when its supplied base is relative.
    let mut base = base;
    let base_path_storage;
    if !base.is_absolute() {
        let current = operations.current_dir()?;
        base_path_storage = operations.join(current.as_ref(), base)?;
        base = base_path_storage.as_ref();
    }
    let retries = if random_len != 0 {
        crate::NUM_RETRIES
    } else {
        1
    };
    let mut rng = fastrand::Rng::new();
    for i in 0..retries {
        #[cfg(all(
            feature = "getrandom",
            any(windows, unix, target_os = "redox", target_os = "wasi")
        ))]
        if i == 3 {
            if let Ok(seed) = getrandom::u64() {
                rng.seed(seed);
            }
        }
        let _ = i;
        let name = tmpname(operations, &mut rng, prefix, suffix, random_len)?;
        let path = operations.join(base, Path::new(name.as_os_str()))?;
        // Original temporary name dies after its join has copied its bytes.
        drop(name);
        match construct(path, operations) {
            Err(error)
                if retries > 1
                    && matches!(
                        O::error_kind(&error),
                        Some(std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::AddrInUse)
                    ) =>
            {
                continue;
            }
            result => return result,
        }
    }
    Err(operations.exhausted(base))
}
