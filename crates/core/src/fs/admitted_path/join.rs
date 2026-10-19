//! General native path joining pays its real Windows normalization scratch first.
use super::{AdmittedPath, PathAdmissionError};
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path},
};

/// Prepared native PathBuf::push semantics over borrowed source components.
/// The temporary component vector, when needed, drops before its supplied credit.
pub struct PreparedPathJoin<'a, ScratchCredit> {
    mode: JoinMode<'a>,
    _scratch_credit: ScratchCredit,
}
enum JoinMode<'a> {
    Right(&'a Path),
    Rooted {
        prefix: Option<&'a OsStr>,
        right: &'a Path,
    },
    Relative {
        base: &'a Path,
        right: &'a Path,
        separator: bool,
    },
    Normalized(Vec<Component<'a>>),
}
impl<'a, ScratchCredit> PreparedPathJoin<'a, ScratchCredit> {
    /// Exact requested temporary Vec capacity; plain/native relative joins need none.
    pub fn scratch_bytes(base: &Path, right: &Path) -> Result<usize, PathAdmissionError> {
        if normalizes(base, right) {
            base.components()
                .count()
                .checked_add(right.components().count())
                .and_then(|capacity| capacity.checked_mul(std::mem::size_of::<Component<'_>>()))
                .ok_or(PathAdmissionError::Arithmetic)
        } else {
            Ok(0)
        }
    }
    /// Caller prepays scratch_bytes before construction. Source paths stay borrowed.
    pub fn new(
        base: &'a Path,
        right: &'a Path,
        scratch_credit: ScratchCredit,
    ) -> Result<Self, PathAdmissionError> {
        let right_prefix = prefix(right);
        let mode = if right.is_absolute() || right_prefix.is_some() {
            JoinMode::Right(right)
        } else if normalizes(base, right) {
            let capacity = Self::scratch_bytes(base, right)? / std::mem::size_of::<Component<'_>>();
            let mut components = Vec::new();
            components.try_reserve_exact(capacity)?;
            if components.capacity() != capacity {
                return Err(PathAdmissionError::CapacityProfile);
            }
            components.extend(base.components());
            for component in right.components() {
                match component {
                    Component::RootDir => {
                        components.truncate(1);
                        components.push(component);
                    }
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if matches!(components.last(), Some(Component::Normal(_))) {
                            components.pop();
                        }
                    }
                    _ => components.push(component),
                }
            }
            JoinMode::Normalized(components)
        } else if right.has_root() {
            JoinMode::Rooted {
                prefix: prefix(base),
                right,
            }
        } else {
            JoinMode::Relative {
                base,
                right,
                separator: needs_separator(base),
            }
        };
        Ok(Self {
            mode,
            _scratch_credit: scratch_credit,
        })
    }
    pub fn capacity(&self) -> Result<usize, PathAdmissionError> {
        let mut capacity = 0usize;
        self.mode.visit(|part| {
            capacity = capacity
                .checked_add(part.as_encoded_bytes().len())
                .ok_or(PathAdmissionError::Arithmetic)?;
            Ok(())
        })?;
        Ok(capacity)
    }
    /// Fill an already admitted, empty exact-capacity target without growing it.
    /// The source and any normalization scratch remain borrowed until completion.
    pub fn fill(&self, target: &mut OsString) -> Result<(), PathAdmissionError> {
        let capacity = self.capacity()?;
        if !target.is_empty() || target.capacity() != capacity {
            return Err(PathAdmissionError::SourceMismatch);
        }
        self.mode.visit(|part| {
            let required = target
                .len()
                .checked_add(part.as_encoded_bytes().len())
                .ok_or(PathAdmissionError::Arithmetic)?;
            if required > capacity {
                return Err(PathAdmissionError::CapacityProfile);
            }
            target.push(part);
            Ok(())
        })?;
        if target.len() != capacity || target.capacity() != capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        Ok(())
    }
    /// Prepay capacity() while this actual source/scratch remains alive.
    pub fn allocate<Credit>(
        self,
        credit: Credit,
    ) -> Result<AdmittedPath<Credit>, PathAdmissionError> {
        let capacity = self.capacity()?;
        let mut encoded = OsString::new();
        encoded.try_reserve_exact(capacity)?;
        if encoded.capacity() != capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        self.fill(&mut encoded)?;
        Ok(AdmittedPath {
            path: encoded.into(),
            credit,
        })
    }
}
impl JoinMode<'_> {
    fn visit(
        &self,
        mut emit: impl FnMut(&OsStr) -> Result<(), PathAdmissionError>,
    ) -> Result<(), PathAdmissionError> {
        match self {
            Self::Right(right) => emit(right.as_os_str()),
            Self::Rooted { prefix, right } => {
                if let Some(prefix) = prefix {
                    emit(prefix)?;
                }
                emit(right.as_os_str())
            }
            Self::Relative {
                base,
                right,
                separator,
            } => {
                emit(base.as_os_str())?;
                if *separator {
                    emit(OsStr::new(std::path::MAIN_SEPARATOR_STR))?;
                }
                emit(right.as_os_str())
            }
            Self::Normalized(components) => {
                let mut separator = false;
                for component in components {
                    if separator && *component != Component::RootDir {
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
                Ok(())
            }
        }
    }
}
fn prefix(path: &Path) -> Option<&OsStr> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => Some(prefix.as_os_str()),
        _ => None,
    }
}
fn normalizes(base: &Path, right: &Path) -> bool {
    !right.is_absolute()
        && prefix(right).is_none()
        && !right.as_os_str().is_empty()
        && matches!(base.components().next(), Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim())
}
fn needs_separator(base: &Path) -> bool {
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
        .is_some_and(|byte| *byte != b'/' && !(cfg!(windows) && *byte == b'\\'))
}
