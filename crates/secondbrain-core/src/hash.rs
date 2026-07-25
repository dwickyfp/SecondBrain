use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const HASH_BYTES: usize = 32;
const HASH_HEX_CHARS: usize = HASH_BYTES * 2;

/// A BLAKE3 digest of exact content bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentHash([u8; HASH_BYTES]);

impl ContentHash {
    /// Computes the BLAKE3 digest of the supplied bytes.
    #[must_use]
    pub fn digest(bytes: impl AsRef<[u8]>) -> Self {
        Self(*blake3::hash(bytes.as_ref()).as_bytes())
    }

    /// Returns the 32 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; HASH_BYTES] {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentHash {
    type Err = ContentHashParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != HASH_HEX_CHARS {
            return Err(ContentHashParseError::InvalidLength {
                actual: value.len(),
            });
        }

        let mut bytes = [0; HASH_BYTES];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high_index = index * 2;
            let high = decode_hex(pair[0], high_index)?;
            let low = decode_hex(pair[1], high_index + 1)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for ContentHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hash = String::deserialize(deserializer)?;
        hash.parse().map_err(serde::de::Error::custom)
    }
}

/// Why a content hash string could not be parsed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContentHashParseError {
    /// The string was not exactly 64 bytes long.
    InvalidLength {
        /// The actual byte length.
        actual: usize,
    },
    /// A character was not lowercase hexadecimal.
    InvalidHex {
        /// The zero-based byte index of the invalid character.
        index: usize,
        /// The invalid byte.
        byte: u8,
    },
}

impl fmt::Display for ContentHashParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { actual } => write!(
                formatter,
                "content hash must be {HASH_HEX_CHARS} hexadecimal characters, got {actual}"
            ),
            Self::InvalidHex { index, byte } => write!(
                formatter,
                "content hash contains invalid lowercase hexadecimal byte {byte:#04x} at index {index}"
            ),
        }
    }
}

impl Error for ContentHashParseError {}

fn decode_hex(byte: u8, index: usize) -> Result<u8, ContentHashParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ContentHashParseError::InvalidHex { index, byte }),
    }
}
