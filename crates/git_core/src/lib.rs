//! Host-agnostic git engine: a persistent registry of local repositories and file diffs.

pub mod diff;
pub mod repo_registry;
#[cfg(test)]
mod test_utils;
