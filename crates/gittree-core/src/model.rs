use std::sync::Arc;

/// A Git object ID as lowercase hex.
pub type Oid = Arc<str>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: Oid,
    /// First parent first.
    pub parents: Vec<Oid>,
    pub author_name: Arc<str>,
    pub author_email: Arc<str>,
    /// Unix seconds, UTC.
    pub author_time: i64,
    pub summary: Arc<str>,
}
