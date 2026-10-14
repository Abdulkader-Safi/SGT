#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitErrorKind {
    GitNotFound,
    GitTooOld,
    NotARepository,
    Untrusted,
    IndexLocked,
    Conflict,
    DirtyWorkTree,
    NonFastForward,
    AuthFailed,
    Network,
    RefExists,
    RefNotFound,
    PatchDoesNotApply,
    /// The commit was to be signed (GPG, SSH or X.509) and signing failed.
    SigningFailed,
    Cancelled,
    Other,
}

impl GitErrorKind {
    pub const ALL: [Self; 16] = [
        Self::GitNotFound,
        Self::GitTooOld,
        Self::NotARepository,
        Self::Untrusted,
        Self::IndexLocked,
        Self::Conflict,
        Self::DirtyWorkTree,
        Self::NonFastForward,
        Self::AuthFailed,
        Self::Network,
        Self::RefExists,
        Self::RefNotFound,
        Self::PatchDoesNotApply,
        Self::SigningFailed,
        Self::Cancelled,
        Self::Other,
    ];
}

/// A failed Git call: what was run and what Git said.
#[derive(Debug, Clone, thiserror::Error)]
#[error("git {} failed ({kind:?}): {}", .args.join(" "), .stderr.trim_end())]
pub struct GitError {
    pub kind: GitErrorKind,
    /// The arguments, with any password in a URL removed.
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stderr: String,
}

impl GitError {
    /// The refusal of a change to a repository that is not trusted, made
    /// before anything runs.
    pub fn untrusted() -> Self {
        Self {
            kind: GitErrorKind::Untrusted,
            args: Vec::new(),
            exit_code: None,
            stderr: String::new(),
        }
    }
}

/// Works out the kind of a failed call from Git's error text. Git gives no
/// machine-readable reason for these, so this reads stderr, which is always
/// in English because every call sets `LC_ALL=C`.
pub fn classify(stderr: &str) -> GitErrorKind {
    use GitErrorKind::*;
    const PATTERNS: &[(&str, GitErrorKind)] = &[
        // First: a signing error can mention a key "not found".
        ("failed to sign the data", SigningFailed),
        ("gpgsm failed", SigningFailed),
        ("Couldn't load public key", SigningFailed),
        ("No private key found", SigningFailed),
        ("user.signingkey", SigningFailed),
        ("not a git repository", NotARepository),
        ("index.lock", IndexLocked),
        ("patch does not apply", PatchDoesNotApply),
        ("patch failed", PatchDoesNotApply),
        ("Authentication failed", AuthFailed),
        ("Permission denied (publickey", AuthFailed),
        ("could not read Username", AuthFailed),
        ("could not read Password", AuthFailed),
        ("terminal prompts disabled", AuthFailed),
        ("Could not resolve host", Network),
        ("Connection refused", Network),
        ("Connection timed out", Network),
        ("Network is unreachable", Network),
        ("Could not read from remote repository", Network),
        ("unable to access", Network),
        ("non-fast-forward", NonFastForward),
        ("fetch first", NonFastForward),
        ("Not possible to fast-forward", NonFastForward),
        ("stale info", NonFastForward),
        ("would be overwritten", DirtyWorkTree),
        ("Please commit your changes or stash them", DirtyWorkTree),
        ("CONFLICT", Conflict),
        ("Merge conflict", Conflict),
        ("fix conflicts", Conflict),
        ("unmerged files", Conflict),
        ("already exists", RefExists),
        ("did not match any", RefNotFound),
        ("unknown revision", RefNotFound),
        ("invalid reference", RefNotFound),
        ("not a valid ref", RefNotFound),
        ("not found", RefNotFound),
    ];
    PATTERNS
        .iter()
        .find(|(pattern, _)| stderr.contains(pattern))
        .map_or(Other, |&(_, kind)| kind)
}

#[cfg(test)]
mod tests {
    use super::GitErrorKind::*;
    use super::*;

    #[test]
    fn classifies_git_error_text() {
        let cases = [
            (
                "fatal: not a git repository (or any of the parent directories): .git",
                NotARepository,
            ),
            (
                "fatal: Unable to create '/r/.git/index.lock': File exists.",
                IndexLocked,
            ),
            (
                "error: patch failed: a.txt:1\nerror: a.txt: patch does not apply",
                PatchDoesNotApply,
            ),
            (
                "fatal: Authentication failed for 'https://example.com/r.git/'",
                AuthFailed,
            ),
            (
                "git@example.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
                AuthFailed,
            ),
            (
                "fatal: could not read Username for 'https://example.com': terminal prompts disabled",
                AuthFailed,
            ),
            (
                "fatal: unable to access 'https://nope.invalid/': Could not resolve host: nope.invalid",
                Network,
            ),
            (
                " ! [rejected]        main -> main (non-fast-forward)",
                NonFastForward,
            ),
            (
                "fatal: Not possible to fast-forward, aborting.",
                NonFastForward,
            ),
            (
                "error: Your local changes to the following files would be overwritten by checkout:",
                DirtyWorkTree,
            ),
            ("CONFLICT (content): Merge conflict in a.txt", Conflict),
            ("fatal: a branch named 'main' already exists", RefExists),
            (
                "error: pathspec 'nope' did not match any file(s) known to git",
                RefNotFound,
            ),
            ("error: branch 'nope' not found", RefNotFound),
            (
                "error: gpg failed to sign the data\nfatal: failed to write commit object",
                SigningFailed,
            ),
            (
                "error: Couldn't load public key /k.pub: No such file or directory?\n\nfatal: failed to write commit object",
                SigningFailed,
            ),
            ("fatal: something nobody expected", Other),
            ("", Other),
        ];
        for (stderr, kind) in cases {
            assert_eq!(classify(stderr), kind, "{stderr}");
        }
    }
}
