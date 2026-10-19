//! Pinned Rust1.98 supported Windows from_wide requested-allocation profile.
use super::PathAdmissionError;

/// Source-derived requested capacities for the supported std UTF-16 constructor.
/// Caller separately owns/pays its original UTF-16 source throughout conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidePathPlan {
    source_units: usize,
    encoded_bytes: usize,
    final_capacity: usize,
    peak_bytes: usize,
}
impl WidePathPlan {
    /// Mirror the pinned standard-library allocation decisions without allocating.
    /// This models requested capacities, excluding native allocator/RSS overhead.
    pub fn new(wide: &[u16]) -> Result<Self, PathAdmissionError> {
        let mut capacity = wide.len();
        let mut encoded = 0usize;
        let mut peak = capacity;
        for scalar in char::decode_utf16(wide.iter().copied()) {
            let bytes = scalar.map_or(3, char::len_utf8);
            encoded = encoded
                .checked_add(bytes)
                .ok_or(PathAdmissionError::Arithmetic)?;
            if encoded > capacity {
                let next = capacity
                    .checked_mul(2)
                    .ok_or(PathAdmissionError::Arithmetic)?
                    .max(encoded)
                    .max(8);
                if next > isize::MAX as usize {
                    return Err(PathAdmissionError::Arithmetic);
                }
                peak = peak.max(
                    capacity
                        .checked_add(next)
                        .ok_or(PathAdmissionError::Arithmetic)?,
                );
                capacity = next;
            }
        }
        Ok(Self {
            source_units: wide.len(),
            encoded_bytes: encoded,
            final_capacity: capacity,
            peak_bytes: peak,
        })
    }
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
    pub fn final_capacity(&self) -> usize {
        self.final_capacity
    }
    pub fn peak_bytes(&self) -> usize {
        self.peak_bytes
    }
    pub fn temporary_bytes(&self) -> usize {
        self.peak_bytes - self.final_capacity
    }
}

#[cfg(windows)]
impl WidePathPlan {
    /// Prepay final_capacity in `credit` and temporary_bytes in `temporary` first.
    /// The latter can be refunded only after this call has physically retired all
    /// reallocations. `OsString::from_wide` preserves unpaired UTF-16 surrogates.
    pub fn allocate<Credit, Temporary>(
        self,
        wide: &[u16],
        credit: Credit,
        _temporary: &Temporary,
    ) -> Result<super::AdmittedPath<Credit>, PathAdmissionError> {
        use std::os::windows::ffi::OsStringExt;
        if WidePathPlan::new(wide)? != self {
            return Err(PathAdmissionError::SourceMismatch);
        }
        let encoded = std::ffi::OsString::from_wide(wide);
        if encoded.capacity() != self.final_capacity || encoded.len() != self.encoded_bytes {
            return Err(PathAdmissionError::CapacityProfile);
        }
        Ok(super::AdmittedPath {
            path: encoded.into(),
            credit,
        })
    }
}
