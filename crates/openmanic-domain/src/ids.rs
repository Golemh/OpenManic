//! Opaque, storage-independent identifiers for stable domain entities.

use core::{fmt, marker::PhantomData};

/// A stable 128-bit domain identifier whose kind prevents accidental mixing.
///
/// The application generates the bytes before persistence. Storage may encode them as
/// `BLOB(16)`, while interchange uses [`OpaqueId::to_lowercase_hex`]. This type deliberately
/// does not expose a SQLite row identifier or choose an identifier generator.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpaqueId<Kind> {
    bytes: [u8; 16],
    kind: PhantomData<fn() -> Kind>,
}

impl<Kind> OpaqueId<Kind> {
    /// Builds an opaque ID from a caller-generated, stable 16-byte value.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self {
            bytes,
            kind: PhantomData,
        }
    }

    /// Returns the exact 16-byte storage representation.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.bytes
    }

    /// Encodes the identifier as 32 lowercase hexadecimal characters for interchange.
    #[must_use]
    pub fn to_lowercase_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(32);
        for byte in self.bytes {
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        text
    }

    /// Decodes the canonical 32-character lowercase hexadecimal interchange representation.
    ///
    /// # Errors
    ///
    /// Returns [`OpaqueIdParseError`] when the input is not exactly 32 lowercase hexadecimal
    /// characters.
    pub fn parse_lowercase_hex(text: &str) -> Result<Self, OpaqueIdParseError> {
        let bytes = text.as_bytes();
        if bytes.len() != 32 {
            return Err(OpaqueIdParseError::WrongLength {
                actual: bytes.len(),
            });
        }
        let mut value = [0_u8; 16];
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            let high = decode_lowercase_hex(pair[0])
                .ok_or(OpaqueIdParseError::InvalidCharacter { index: index * 2 })?;
            let low =
                decode_lowercase_hex(pair[1]).ok_or(OpaqueIdParseError::InvalidCharacter {
                    index: index * 2 + 1,
                })?;
            value[index] = (high << 4) | low;
        }
        Ok(Self::from_bytes(value))
    }
}

impl<Kind> fmt::Debug for OpaqueId<Kind> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OpaqueId")
            .field(&self.to_lowercase_hex())
            .finish()
    }
}

fn decode_lowercase_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Failure while parsing the documented stable-ID interchange format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpaqueIdParseError {
    /// The text was not the required 32-character hexadecimal length.
    WrongLength {
        /// Number of UTF-8 bytes supplied by the caller.
        actual: usize,
    },
    /// A character was not lowercase hexadecimal.
    InvalidCharacter {
        /// Zero-based byte position of the invalid character.
        index: usize,
    },
}

impl fmt::Display for OpaqueIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { actual } => {
                write!(
                    formatter,
                    "opaque ID must contain 32 lowercase hexadecimal characters, got {actual}"
                )
            }
            Self::InvalidCharacter { index } => {
                write!(
                    formatter,
                    "opaque ID contains a non-lowercase-hexadecimal character at {index}"
                )
            }
        }
    }
}

impl std::error::Error for OpaqueIdParseError {}

/// Marker for a stable category identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CategoryIdKind {}

/// Stable opaque identifier for a category.
pub type CategoryId = OpaqueId<CategoryIdKind>;

/// Marker for a stable application identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ApplicationIdKind {}

/// Stable opaque identifier for an application.
pub type ApplicationId = OpaqueId<ApplicationIdKind>;

/// Marker for a stable tracker-run identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TrackerRunIdKind {}

/// Stable opaque identifier for one tracker run.
pub type TrackerRunId = OpaqueId<TrackerRunIdKind>;

/// Marker for a stable focus-session identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FocusSessionIdKind {}

/// Stable opaque identifier for one focus session.
pub type FocusSessionId = OpaqueId<FocusSessionIdKind>;

/// Marker for a stable repeating-schedule series identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScheduleSeriesIdKind {}

/// Stable opaque identifier for one recurring schedule lineage.
pub type ScheduleSeriesId = OpaqueId<ScheduleSeriesIdKind>;

/// Marker for a stable one-time schedule identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OneTimeScheduleIdKind {}

/// Stable opaque identifier for one one-time schedule item.
pub type OneTimeScheduleId = OpaqueId<OneTimeScheduleIdKind>;

#[cfg(test)]
mod tests {
    use super::{
        ApplicationId, CategoryId, FocusSessionId, OneTimeScheduleId, OpaqueIdParseError,
        ScheduleSeriesId, TrackerRunId,
    };

    #[test]
    fn stable_ids_round_trip_exact_bytes_with_lowercase_encoding() {
        let category = CategoryId::from_bytes([0x0f; 16]);
        let application = ApplicationId::from_bytes([0xf0; 16]);
        let tracker_run = TrackerRunId::from_bytes([0x12; 16]);
        let focus_session = FocusSessionId::from_bytes([0x21; 16]);
        let schedule_series = ScheduleSeriesId::from_bytes([0x32; 16]);
        let one_time_schedule = OneTimeScheduleId::from_bytes([0x23; 16]);

        assert_eq!(
            category.to_lowercase_hex(),
            "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
        );
        assert_eq!(
            application.to_lowercase_hex(),
            "f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0"
        );
        assert_eq!(tracker_run.as_bytes(), [0x12; 16]);
        assert_eq!(focus_session.as_bytes(), [0x21; 16]);
        assert_eq!(schedule_series.as_bytes(), [0x32; 16]);
        assert_eq!(one_time_schedule.as_bytes(), [0x23; 16]);
        assert_eq!(
            CategoryId::parse_lowercase_hex(&category.to_lowercase_hex()),
            Ok(category)
        );
    }

    #[test]
    fn stable_id_parser_rejects_noncanonical_forms() {
        assert_eq!(
            CategoryId::parse_lowercase_hex("abc"),
            Err(OpaqueIdParseError::WrongLength { actual: 3 })
        );
        assert_eq!(
            CategoryId::parse_lowercase_hex("0000000000000000000000000000000G"),
            Err(OpaqueIdParseError::InvalidCharacter { index: 31 })
        );
    }
}
