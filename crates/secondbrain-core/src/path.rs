use std::error::Error;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A normalized, UTF-8, workspace-relative path using `/` separators.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspacePath(String);

impl WorkspacePath {
    /// Validates and constructs a portable workspace path.
    ///
    /// Backslashes are rejected rather than interpreted as separators so that
    /// the same serialized path has the same meaning on every platform.
    pub fn new(path: impl AsRef<str>) -> Result<Self, WorkspacePathError> {
        let path = path.as_ref();
        validate(path)?;
        Ok(Self(path.to_owned()))
    }

    /// Returns this portable path as a platform `Path`.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    /// Returns this portable path as UTF-8 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspacePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for WorkspacePath {
    type Err = WorkspacePathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for WorkspacePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorkspacePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::new(path).map_err(serde::de::Error::custom)
    }
}

/// Why a portable workspace path was rejected.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkspacePathError {
    /// The path has no content.
    Empty,
    /// The path is absolute or contains a Windows prefix form.
    Absolute,
    /// The path contains a parent (`..`) component.
    ParentTraversal,
    /// The path contains a NUL byte.
    Nul,
    /// The path contains a backslash, which is not a portable separator.
    Backslash,
    /// The path is not in normalized lexical form.
    NotNormalized,
    /// The first component is reserved for SecondBrain metadata.
    Reserved,
}

impl fmt::Display for WorkspacePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "workspace path cannot be empty",
            Self::Absolute => "workspace path must be relative and have no platform prefix",
            Self::ParentTraversal => "workspace path cannot contain a parent component",
            Self::Nul => "workspace path cannot contain NUL",
            Self::Backslash => "workspace path must use '/' separators, not backslashes",
            Self::NotNormalized => "workspace path must be lexically normalized",
            Self::Reserved => {
                "workspace path cannot start with the reserved '.secondbrain' component"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for WorkspacePathError {}

fn validate(path: &str) -> Result<(), WorkspacePathError> {
    if path.is_empty() {
        return Err(WorkspacePathError::Empty);
    }
    if path.contains('\0') {
        return Err(WorkspacePathError::Nul);
    }
    if path.contains('\\') {
        return Err(WorkspacePathError::Backslash);
    }
    if is_absolute_or_prefixed(path) {
        return Err(WorkspacePathError::Absolute);
    }

    let mut components = path.split('/');
    let first = components.next().ok_or(WorkspacePathError::Empty)?;
    if first == ".secondbrain" {
        return Err(WorkspacePathError::Reserved);
    }

    for component in std::iter::once(first).chain(components) {
        if component == ".." {
            return Err(WorkspacePathError::ParentTraversal);
        }
        if component.is_empty() || component == "." {
            return Err(WorkspacePathError::NotNormalized);
        }
    }

    Ok(())
}

fn is_absolute_or_prefixed(path: &str) -> bool {
    path.starts_with('/') || has_windows_drive_prefix(path)
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
