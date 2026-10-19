//! Native single-leaf assembly preserves std path semantics without hidden scratch vectors.
use super::{AdmittedPath, PathAdmissionError};
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path},
};

/// An allocation-free plan borrowing the actual native base and ordinary leaf.
pub struct LeafPathPlan<'a> {
    base: &'a Path,
    leaf: &'a OsStr,
    capacity: usize,
    normalized: bool,
}
impl<'a> LeafPathPlan<'a> {
    pub fn new(base: &'a Path, leaf: &'a OsStr) -> Result<Self, PathAdmissionError> {
        let mut components = Path::new(leaf).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(PathAdmissionError::InvalidLeaf);
        }
        let mut plan = Self {
            base,
            leaf,
            capacity: 0,
            normalized: false,
        };
        #[cfg(windows)]
        {
            plan.normalized = matches!(base.components().next(), Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim());
        }
        let mut capacity = 0usize;
        plan.visit(|part| {
            capacity = capacity
                .checked_add(part.as_encoded_bytes().len())
                .ok_or(PathAdmissionError::Arithmetic)?;
            Ok(())
        })?;
        plan.capacity = capacity;
        Ok(plan)
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Prepay capacity() before allocation. Original source spelling stays borrowed.
    pub fn allocate<Credit>(
        self,
        credit: Credit,
    ) -> Result<AdmittedPath<Credit>, PathAdmissionError> {
        let mut encoded = OsString::new();
        encoded.try_reserve_exact(self.capacity)?;
        if encoded.capacity() != self.capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        self.visit(|part| {
            encoded.push(part);
            Ok(())
        })?;
        if encoded.len() != self.capacity || encoded.capacity() != self.capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        Ok(AdmittedPath {
            path: encoded.into(),
            credit,
        })
    }
    fn visit(
        &self,
        mut emit: impl FnMut(&OsStr) -> Result<(), PathAdmissionError>,
    ) -> Result<(), PathAdmissionError> {
        if self.normalized {
            // Pinned std PathBuf::_push collects the base components and appends
            // the supplied components. Our new leaf is a single Normal component,
            // so this exact iteration needs neither its temporary Vec nor a stack.
            let mut separator = false;
            for component in self
                .base
                .components()
                .chain(std::iter::once(Component::Normal(self.leaf)))
            {
                if separator && component != Component::RootDir {
                    emit(OsStr::new(std::path::MAIN_SEPARATOR_STR))?;
                }
                emit(component.as_os_str())?;
                separator = match component {
                    Component::RootDir => false,
                    Component::Prefix(prefix) => {
                        !matches!(
                            prefix.kind(),
                            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                        ) && !prefix.as_os_str().is_empty()
                    }
                    _ => true,
                };
            }
        } else {
            emit(self.base.as_os_str())?;
            let separator = needs_separator(self.base);
            if separator {
                emit(OsStr::new(std::path::MAIN_SEPARATOR_STR))?;
            }
            emit(self.leaf)?;
        }
        Ok(())
    }
}
fn is_separator(byte: u8) -> bool {
    byte == b'/' || (cfg!(windows) && byte == b'\\')
}

fn needs_separator(base: &Path) -> bool {
    #[cfg(windows)]
    if let Some(Component::Prefix(prefix)) = base.components().next()
        && prefix.as_os_str() == base.as_os_str()
        && matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        )
    {
        return false;
    }
    base.as_os_str()
        .as_encoded_bytes()
        .last()
        .is_some_and(|byte| !is_separator(*byte))
}
