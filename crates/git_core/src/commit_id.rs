//! The id of a commit, without exposing `git2::Oid` in the public API.

use git2::Oid;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};
use thiserror::Error;

/// Length of a full (SHA-1) object id in hexadecimal characters.
const HEX_LEN: usize = 40;
/// Number of characters [`CommitId::short`] shows.
const SHORT_LEN: usize = 7;

/// Identifies a commit: a 40 character hexadecimal SHA-1.
///
/// Displays and serializes as lowercase hex. Parsing accepts upper and lower case but requires
/// the full id, so a mistyped prefix can't silently turn into a different commit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitId(Oid);

impl CommitId {
    pub(crate) fn from_oid(oid: Oid) -> Self {
        Self(oid)
    }

    /// The first 7 characters, as shown in most git UIs. Not guaranteed to be unique.
    pub fn short(&self) -> String {
        self.0.to_string()[..SHORT_LEN].to_owned()
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0.to_string())
    }
}

impl fmt::Debug for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitId({})", self.0)
    }
}

/// `s` is not a full 40 character hexadecimal commit id.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid commit id `{0}`: expected 40 hexadecimal characters")]
pub struct ParseCommitIdError(pub String);

impl FromStr for CommitId {
    type Err = ParseCommitIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s.len() == HEX_LEN && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid {
            return Err(ParseCommitIdError(s.to_owned()));
        }
        Oid::from_str(s)
            .map(Self)
            .map_err(|_| ParseCommitIdError(s.to_owned()))
    }
}

impl Serialize for CommitId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CommitId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}
