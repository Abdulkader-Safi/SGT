use crate::model::{Ref, RefKind};

/// The `git for-each-ref --format` that `parse_refs` reads: NUL between
/// fields, one ref per line.
pub const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(upstream)%00%(upstream:track,nobracket)%00%(HEAD)";

pub fn parse_refs(out: &[u8]) -> Vec<Ref> {
    out.split(|&b| b == b'\n').filter_map(parse_ref).collect()
}

fn parse_ref(line: &[u8]) -> Option<Ref> {
    let line = String::from_utf8_lossy(line);
    let mut fields = line.split('\0');
    let name = fields.next()?.to_owned();
    let object = fields.next()?;
    let peeled = fields.next()?;
    let upstream = fields.next()?;
    let track = fields.next()?;
    let is_current = fields.next()? == "*";

    let kind = if name.starts_with("refs/heads/") {
        RefKind::Local
    } else if name.starts_with("refs/remotes/") {
        // origin/HEAD only says which branch the remote checks out.
        if name.ends_with("/HEAD") {
            return None;
        }
        RefKind::Remote
    } else if name.starts_with("refs/tags/") {
        RefKind::Tag
    } else if name == "refs/stash" {
        RefKind::Stash
    } else {
        // Snapshot refs and anything else the app does not show.
        return None;
    };
    // "ahead 2, behind 1", "ahead 2", "behind 1", "gone" or nothing.
    let count = |word: &str| {
        track
            .split(", ")
            .find_map(|part| part.strip_prefix(word)?.trim().parse().ok())
            .unwrap_or(0)
    };
    Some(Ref {
        kind,
        // An annotated tag has a peeled ID: the commit it points at.
        target: if peeled.is_empty() { object } else { peeled }.into(),
        upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
        ahead: count("ahead"),
        behind: count("behind"),
        is_current,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind_of_ref() {
        let out = b"refs/heads/main\0aaa\0\0refs/remotes/origin/main\0ahead 2, behind 1\0*\n\
refs/heads/topic\0bbb\0\0\0\0 \n\
refs/heads/old\0bbb\0\0refs/remotes/origin/old\0gone\0 \n\
refs/remotes/origin/HEAD\0aaa\0\0\0\0 \n\
refs/remotes/origin/main\0ccc\0\0\0\0 \n\
refs/tags/v1\0ddd\0aaa\0\0\0 \n\
refs/tags/light\0aaa\0\0\0\0 \n\
refs/stash\0eee\0\0\0\0 \n\
refs/gittree/snapshots/x\0fff\0\0\0\0 \n";
        let refs = parse_refs(out);
        let names: Vec<&str> = refs.iter().map(|r| r.short_name()).collect();
        assert_eq!(
            names,
            [
                "main",
                "topic",
                "old",
                "origin/main",
                "v1",
                "light",
                "stash"
            ]
        );
        let main = &refs[0];
        assert!(main.is_current);
        assert_eq!((main.ahead, main.behind), (2, 1));
        assert_eq!(main.upstream.as_deref(), Some("refs/remotes/origin/main"));
        assert_eq!((refs[2].ahead, refs[2].behind), (0, 0));
        assert_eq!(refs[3].kind, RefKind::Remote);
        // The annotated tag points at its commit, not at the tag object.
        assert_eq!((&*refs[4].target, refs[4].kind), ("aaa", RefKind::Tag));
        assert_eq!(refs[6].kind, RefKind::Stash);
    }
}
