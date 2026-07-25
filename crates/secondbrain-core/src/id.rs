use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use ulid::Ulid;

/// An error returned when a typed domain ID is not a valid canonical ULID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IdParseError {
    source: Option<ulid::DecodeError>,
}

impl IdParseError {
    const fn noncanonical() -> Self {
        Self { source: None }
    }
}

impl fmt::Display for IdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.source {
            Some(source) => write!(formatter, "invalid ULID: {source}"),
            None => formatter.write_str("invalid ULID: expected canonical uppercase text"),
        }
    }
}

impl Error for IdParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<ulid::DecodeError> for IdParseError {
    fn from(source: ulid::DecodeError) -> Self {
        Self {
            source: Some(source),
        }
    }
}

fn parse_canonical_ulid(value: &str) -> Result<Ulid, IdParseError> {
    let parsed = value.parse::<Ulid>().map_err(IdParseError::from)?;
    if value.as_bytes() != parsed.to_string().as_bytes() {
        return Err(IdParseError::noncanonical());
    }

    Ok(parsed)
}

macro_rules! define_ulid_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Generates a new unique identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Ulid::generate())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_canonical_ulid(value).map(Self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

define_ulid_id!(WorkspaceId, "The stable identity of a workspace.");
define_ulid_id!(NoteId, "The stable identity of a note.");
define_ulid_id!(
    TransactionId,
    "The stable identity of a workspace transaction."
);

macro_rules! define_counter {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Constructs a counter from its stored value.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the stored counter value.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Returns the next counter value, or `None` at `u64::MAX`.
            #[must_use]
            pub const fn checked_increment(self) -> Option<Self> {
                match self.0.checked_add(1) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }
        }
    };
}

define_counter!(WorkspaceEpoch, "A monotonically checked workspace epoch.");
define_counter!(NoteVersion, "A monotonically checked note version.");
