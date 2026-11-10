use super::{LocalFsError, LocalResult};
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);
impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> LocalResult<()> {
        if self.is_cancelled() {
            Err(LocalFsError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DirectoryIdentity(Identity);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Identity {
    Native(u64, u64),
    Portable(PathBuf),
}
impl DirectoryIdentity {
    /// Whether this supplied identity denotes retained native-directory semantics.
    /// Portable location identities do not prove held-ancestor safety.
    pub fn is_held(&self) -> bool {
        matches!(self.0, Identity::Native(..))
    }
    pub fn native(device: u64, inode: u64) -> Self {
        Self(Identity::Native(device, inode))
    }
    /// Portable fallbacks provide weaker location identity, not Unix handle safety.
    pub fn portable(location: PathBuf) -> Self {
        Self(Identity::Portable(location))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ListingLimits {
    entries: usize,
    name_bytes: usize,
}
impl Default for ListingLimits {
    fn default() -> Self {
        Self {
            entries: 100_000,
            name_bytes: 8 * 1024 * 1024,
        }
    }
}
impl ListingLimits {
    pub fn new(entries: usize, name_bytes: usize) -> LocalResult<Self> {
        if entries == 0 || entries > 100_000 || name_bytes == 0 || name_bytes > 8 * 1024 * 1024 {
            return Err(LocalFsError::BoundExceeded("listing configuration"));
        }
        Ok(Self {
            entries,
            name_bytes,
        })
    }
    pub fn entries(self) -> usize {
        self.entries
    }
    pub fn name_bytes(self) -> usize {
        self.name_bytes
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RemovalLimits {
    depth: usize,
    entries: usize,
}
impl Default for RemovalLimits {
    fn default() -> Self {
        Self {
            depth: 256,
            entries: 1_000_000,
        }
    }
}
impl RemovalLimits {
    pub fn new(depth: usize, entries: usize) -> LocalResult<Self> {
        if depth == 0 || depth > 256 || entries == 0 || entries > 1_000_000 {
            return Err(LocalFsError::BoundExceeded("removal configuration"));
        }
        Ok(Self { depth, entries })
    }
    pub fn depth(self) -> usize {
        self.depth
    }
    pub fn entries(self) -> usize {
        self.entries
    }
}
#[derive(Clone, Debug)]
pub struct ScanPolicy {
    excluded: Vec<OsString>,
    entries: usize,
    depth: usize,
}
impl ScanPolicy {
    pub fn new(excluded: Vec<OsString>, entries: usize, depth: usize) -> LocalResult<Self> {
        if entries == 0 || entries > 1_000_000 || depth == 0 || depth > 256 {
            return Err(LocalFsError::BoundExceeded("statistics configuration"));
        }
        Ok(Self {
            excluded,
            entries,
            depth,
        })
    }
    pub fn enters(&self, name: &OsStr) -> bool {
        !self.excluded.iter().any(|excluded| excluded == name)
    }
    pub fn entries(&self) -> usize {
        self.entries
    }
    pub fn depth(&self) -> usize {
        self.depth
    }
}

pub fn valid_native_name(name: &OsStr) -> bool {
    let path = Path::new(name);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return false;
    }
    if name.as_encoded_bytes().contains(&0) {
        return false;
    }
    #[cfg(windows)]
    {
        let text = name.to_string_lossy();
        if text.contains([':', '/', '\\']) || text.ends_with(['.', ' ']) {
            return false;
        }
        let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        }) {
            return false;
        }
    }
    true
}
