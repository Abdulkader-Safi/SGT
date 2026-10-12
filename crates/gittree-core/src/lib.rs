//! Git access, parsers and graph layout for GitTree. No UI dependency.

pub mod error;
pub mod graph;
pub mod model;
pub mod ops;
pub mod parse;
pub mod runner;

pub use error::{GitError, GitErrorKind};
pub use runner::GitRunner;
