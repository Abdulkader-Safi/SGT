use std::collections::HashSet;
use std::sync::Arc;

use crate::model::Commit;

/// The `--format` that `LogParser` reads. Fields are split by the unit
/// separator and records by NUL (`git log -z`).
pub const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s";

/// Parses `git log` records. Keeps one copy of each author name and email,
/// so reuse one parser for a whole log.
#[derive(Default)]
pub struct LogParser {
    interned: HashSet<Arc<str>>,
}

impl LogParser {
    /// Parses one record, without its NUL terminator. Returns `None` when the
    /// record does not have the six fields.
    pub fn parse(&mut self, record: &[u8]) -> Option<Commit> {
        let mut fields = record.splitn(6, |&b| b == 0x1f);
        let oid = text(fields.next()?);
        let parents = text(fields.next()?);
        let author_name = self.intern(fields.next()?);
        let author_email = self.intern(fields.next()?);
        let author_time = text(fields.next()?).parse().ok()?;
        let summary = text(fields.next()?);
        Some(Commit {
            oid: oid.as_ref().into(),
            parents: parents.split_ascii_whitespace().map(Arc::from).collect(),
            author_name,
            author_email,
            author_time,
            summary: summary.as_ref().into(),
        })
    }

    fn intern(&mut self, bytes: &[u8]) -> Arc<str> {
        let value = text(bytes);
        if let Some(found) = self.interned.get(value.as_ref()) {
            return found.clone();
        }
        let value: Arc<str> = value.as_ref().into();
        self.interned.insert(value.clone());
        value
    }
}

// Git re-encodes log output to UTF-8, but a commit can still carry bad bytes.
fn text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_merge_record() {
        let mut parser = LogParser::default();
        let commit = parser
            .parse(b"aaa\x1fbbb ccc\x1fAda\x1fada@example.com\x1f1700000000\x1fMerge \x1f odd")
            .unwrap();
        assert_eq!(&*commit.oid, "aaa");
        assert_eq!(commit.parents, [Arc::from("bbb"), Arc::from("ccc")]);
        assert_eq!(&*commit.author_name, "Ada");
        assert_eq!(&*commit.author_email, "ada@example.com");
        assert_eq!(commit.author_time, 1_700_000_000);
        // A separator inside the summary stays in the summary.
        assert_eq!(&*commit.summary, "Merge \x1f odd");
    }

    #[test]
    fn parses_a_root_commit_with_an_empty_summary() {
        let commit = LogParser::default()
            .parse(b"aaa\x1f\x1fAda\x1fa@b\x1f1\x1f")
            .unwrap();
        assert!(commit.parents.is_empty());
        assert_eq!(&*commit.summary, "");
    }

    #[test]
    fn shares_author_strings_between_commits() {
        let mut parser = LogParser::default();
        let a = parser.parse(b"a\x1f\x1fAda\x1fa@b\x1f1\x1fone").unwrap();
        let b = parser.parse(b"b\x1fa\x1fAda\x1fa@b\x1f2\x1ftwo").unwrap();
        assert!(Arc::ptr_eq(&a.author_name, &b.author_name));
        assert!(Arc::ptr_eq(&a.author_email, &b.author_email));
    }

    #[test]
    fn rejects_a_short_record() {
        assert!(LogParser::default().parse(b"aaa\x1fbbb").is_none());
    }
}
