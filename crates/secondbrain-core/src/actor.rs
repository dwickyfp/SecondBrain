use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum number of Unicode scalar values in an actor or device identity.
///
/// The 255-character limit bounds identity fields in synchronization envelopes
/// while allowing descriptive human-readable names.
pub const MAX_IDENTITY_LEN: usize = 255;

/// Why an actor or device identity was rejected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IdentityError {
    /// The identity has no characters.
    Empty,
    /// Leading or trailing whitespace would require silent normalization.
    SurroundingWhitespace,
    /// A Unicode control character was present.
    ControlCharacter {
        /// The zero-based Unicode scalar index of the control character.
        index: usize,
        /// The rejected control character.
        character: char,
    },
    /// The identity exceeded [`MAX_IDENTITY_LEN`].
    TooLong {
        /// The maximum number of Unicode scalar values.
        max: usize,
        /// The actual number of Unicode scalar values.
        actual: usize,
    },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identity cannot be empty"),
            Self::SurroundingWhitespace => {
                formatter.write_str("identity cannot have leading or trailing whitespace")
            }
            Self::ControlCharacter { index, character } => write!(
                formatter,
                "identity contains control character {character:?} at index {index}"
            ),
            Self::TooLong { max, actual } => {
                write!(
                    formatter,
                    "identity cannot exceed {max} characters, got {actual}"
                )
            }
        }
    }
}

impl Error for IdentityError {}

fn validate(value: &str) -> Result<(), IdentityError> {
    if value.is_empty() {
        return Err(IdentityError::Empty);
    }
    if value.trim() != value {
        return Err(IdentityError::SurroundingWhitespace);
    }

    for (index, character) in value.chars().enumerate() {
        if character.is_control() {
            return Err(IdentityError::ControlCharacter { index, character });
        }
    }

    let actual = value.chars().count();
    if actual > MAX_IDENTITY_LEN {
        return Err(IdentityError::TooLong {
            max: MAX_IDENTITY_LEN,
            actual,
        });
    }
    Ok(())
}

macro_rules! define_identity {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Validates and constructs an identity without normalization.
            pub fn new(value: impl AsRef<str>) -> Result<Self, IdentityError> {
                let value = value.as_ref();
                validate(value)?;
                Ok(Self(value.to_owned()))
            }

            /// Returns the identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

define_identity!(ActorId, "The validated identity of an operation actor.");
define_identity!(DeviceId, "The validated identity of a source device.");
