//! Git access, parsers and graph layout for Safi Git Tree. No UI dependency.

pub mod error;
pub mod graph;
pub mod journal;
pub mod model;
pub mod ops;
pub mod parse;
pub mod patch;
pub mod runner;
pub mod store;

pub use error::{GitError, GitErrorKind};
pub use runner::{Access, GitRunner};
