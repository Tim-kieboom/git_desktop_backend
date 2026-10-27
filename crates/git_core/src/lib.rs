//! Host-agnostic git engine: a persistent registry of local repositories and file diffs and status.

pub mod diff;
pub mod error;
pub mod repo_registry;
pub mod staging;
pub mod status;
#[cfg(test)]
mod test_utils;
