#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitErrorKind {
    GitNotFound,
    Cancelled,
    Other,
}

/// A failed Git call: what was run and what Git said.
#[derive(Debug, thiserror::Error)]
#[error("git {} failed ({kind:?}): {}", .args.join(" "), .stderr.trim_end())]
pub struct GitError {
    pub kind: GitErrorKind,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stderr: String,
}
