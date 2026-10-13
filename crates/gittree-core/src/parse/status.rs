use crate::model::{Change, ConflictKind, RepoPath, StatusEntry};

/// Parses `git status --porcelain=v2 -z`. Ignored files and header lines
/// are skipped.
pub fn parse_status(out: &[u8]) -> Vec<StatusEntry> {
    let mut records = out.split(|&b| b == 0);
    let mut entries = Vec::new();
    while let Some(record) = records.next() {
        // The path is the last field and may hold spaces, so each kind is
        // split into its fixed number of fields.
        let (fields, has_orig) = match record.first() {
            Some(b'1') => (9, false),
            Some(b'2') => (10, true),
            Some(b'u') => (11, false),
            Some(b'?') => (2, false),
            _ => continue,
        };
        let mut parts = record.splitn(fields, |&b| b == b' ');
        let kind = parts.next().unwrap_or_default();
        let xy = if kind == b"?" {
            b""
        } else {
            parts.next().unwrap_or_default()
        };
        let Some(path) = parts.last() else { continue };
        let orig_path = has_orig
            .then(|| records.next())
            .flatten()
            .map(|path| RepoPath(path.to_vec()));
        let change = |code: Option<&u8>| match code {
            Some(b'A') => Change::Added,
            Some(b'M') => Change::Modified,
            Some(b'D') => Change::Deleted,
            Some(b'R') => Change::Renamed,
            Some(b'C') => Change::Copied,
            Some(b'T') => Change::TypeChange,
            _ => Change::Unmodified,
        };
        let conflict = (kind == b"u").then_some(match xy {
            b"DD" => ConflictKind::BothDeleted,
            b"AU" => ConflictKind::AddedByUs,
            b"UD" => ConflictKind::DeletedByThem,
            b"UA" => ConflictKind::AddedByThem,
            b"DU" => ConflictKind::DeletedByUs,
            b"AA" => ConflictKind::BothAdded,
            _ => ConflictKind::BothModified,
        });
        let (index, worktree) = match (kind, conflict) {
            (b"?", _) => (Change::Unmodified, Change::Untracked),
            (_, Some(_)) => (Change::Unmodified, Change::Unmodified),
            _ => (change(xy.first()), change(xy.get(1))),
        };
        entries.push(StatusEntry {
            path: RepoPath(path.to_vec()),
            orig_path,
            index,
            worktree,
            conflict,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_record_kind() {
        let out = b"# branch.oid abc\0\
1 .M N... 100644 100644 100644 1111 1111 plain.txt\0\
1 A. N... 000000 100644 100644 0000 2222 with space.txt\0\
1 MD N... 100644 100644 000000 1111 3333 both.txt\0\
2 R. N... 100644 100644 100644 1111 1111 R100 new name.txt\0old name.txt\0\
u UU N... 100644 100644 100644 100644 1111 2222 3333 clash.txt\0\
u DU N... 100644 100644 100644 100644 1111 2222 3333 gone.txt\0\
? new\nline.txt\0\
! ignored.txt\0";
        let entries = parse_status(out);
        let shape: Vec<_> = entries
            .iter()
            .map(|e| (e.path.to_string(), e.index, e.worktree, e.conflict))
            .collect();
        use Change::*;
        assert_eq!(
            shape,
            [
                ("plain.txt".to_owned(), Unmodified, Modified, None),
                ("with space.txt".to_owned(), Added, Unmodified, None),
                ("both.txt".to_owned(), Modified, Deleted, None),
                ("new name.txt".to_owned(), Renamed, Unmodified, None),
                (
                    "clash.txt".to_owned(),
                    Unmodified,
                    Unmodified,
                    Some(ConflictKind::BothModified)
                ),
                (
                    "gone.txt".to_owned(),
                    Unmodified,
                    Unmodified,
                    Some(ConflictKind::DeletedByUs)
                ),
                ("new\nline.txt".to_owned(), Unmodified, Untracked, None),
            ]
        );
        assert_eq!(entries[3].orig_path, Some("old name.txt".into()));
        assert!(entries[2].staged() && entries[2].unstaged());
        assert!(!entries[4].staged() && entries[4].unstaged());
    }
}
