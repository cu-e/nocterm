//! Pure format selection. Native storage supplies exact lossless legacy measurement.
use super::{FormatSize, HistoryCodecError, MAX_FILE_BYTES, MAX_RESIDENT_BYTES, MeasurementError};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryEncoding {
    Version2,
    LegacyV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodingDecision {
    pub encoding: HistoryEncoding,
    pub size: FormatSize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryOrigin {
    New,
    LegacyV1,
    Version2,
}
fn bound(size: FormatSize) -> Result<(), HistoryCodecError> {
    if size.encoded_bytes > MAX_FILE_BYTES as usize {
        return Err(HistoryCodecError::EncodedLimit);
    }
    if size.structural_bytes > MAX_RESIDENT_BYTES {
        return Err(HistoryCodecError::StructuralLimit);
    }
    Ok(())
}
/// Resident payload has an independent gate and never acquires a format exemption.
/// JSON/overflow errors and legacy conversion failure are not fallback eligibility.
pub fn select_encoding(
    origin: HistoryOrigin,
    resident_bytes: usize,
    version2: Result<FormatSize, MeasurementError>,
    legacy: impl FnOnce() -> Result<FormatSize, HistoryCodecError>,
) -> Result<EncodingDecision, HistoryCodecError> {
    if resident_bytes > MAX_RESIDENT_BYTES {
        return Err(HistoryCodecError::StructuralLimit);
    }
    let size = version2.map_err(HistoryCodecError::from)?;
    match bound(size) {
        Ok(()) => Ok(EncodingDecision {
            encoding: HistoryEncoding::Version2,
            size,
        }),
        Err(error) if origin != HistoryOrigin::LegacyV1 => Err(error),
        Err(_) => {
            let size = legacy()?;
            bound(size)?;
            Ok(EncodingDecision {
                encoding: HistoryEncoding::LegacyV1,
                size,
            })
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fits() -> FormatSize {
        FormatSize {
            encoded_bytes: MAX_FILE_BYTES as usize,
            structural_bytes: MAX_RESIDENT_BYTES,
        }
    }
    #[test]
    fn imported_format_falls_back_only_for_destination_format_cost() {
        for size in [
            FormatSize {
                encoded_bytes: MAX_FILE_BYTES as usize + 1,
                ..fits()
            },
            FormatSize {
                structural_bytes: MAX_RESIDENT_BYTES + 1,
                ..fits()
            },
        ] {
            let decision = select_encoding(
                HistoryOrigin::LegacyV1,
                MAX_RESIDENT_BYTES,
                Ok(size),
                || Ok(fits()),
            )
            .unwrap();
            assert_eq!(decision.encoding, HistoryEncoding::LegacyV1);
            for origin in [HistoryOrigin::New, HistoryOrigin::Version2] {
                assert!(
                    select_encoding(origin, 0, Ok(size), || panic!("new/v2 cannot fallback"))
                        .is_err()
                );
            }
        }
        assert_eq!(
            select_encoding(
                HistoryOrigin::LegacyV1,
                MAX_RESIDENT_BYTES,
                Ok(fits()),
                || panic!("bounded v2 is preferred")
            )
            .unwrap()
            .encoding,
            HistoryEncoding::Version2
        );
    }
    #[test]
    fn resident_and_overflow_failures_cannot_request_legacy_conversion() {
        assert!(matches!(
            select_encoding(
                HistoryOrigin::LegacyV1,
                MAX_RESIDENT_BYTES + 1,
                Ok(fits()),
                || panic!("resident failure is final")
            ),
            Err(HistoryCodecError::StructuralLimit)
        ));
        assert!(matches!(
            select_encoding(
                HistoryOrigin::LegacyV1,
                0,
                Err(MeasurementError::ArithmeticOverflow),
                || panic!("overflow is final")
            ),
            Err(HistoryCodecError::ArithmeticOverflow)
        ));
    }
    #[test]
    fn legacy_measurement_error_and_legacy_limit_are_preserved() {
        let too_large = FormatSize {
            encoded_bytes: MAX_FILE_BYTES as usize + 1,
            ..fits()
        };
        assert!(matches!(
            select_encoding(HistoryOrigin::LegacyV1, 0, Ok(too_large), || Err(
                HistoryCodecError::ArithmeticOverflow
            )),
            Err(HistoryCodecError::ArithmeticOverflow)
        ));
        assert!(matches!(
            select_encoding(HistoryOrigin::LegacyV1, 0, Ok(too_large), || Ok(too_large)),
            Err(HistoryCodecError::EncodedLimit)
        ));
    }
}
