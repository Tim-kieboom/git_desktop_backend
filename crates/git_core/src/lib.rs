//! Host-agnostic git engine: a persistent registry of local repositories and file diffs.

pub mod diff;
pub mod repo_registry;
pub mod status;
#[cfg(test)]
mod test_utils;
