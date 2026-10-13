//! Builds the patch for part of a file's diff, so single hunks and lines can
//! be staged, unstaged or discarded with `git apply`.

use std::collections::HashSet;

use crate::model::{FileDiff, LineKind};
use crate::ops::read::quote_path;

/// The chosen changed lines, each as (hunk, line within the hunk). Context
/// lines in it are ignored.
pub type Lines = HashSet<(usize, usize)>;

/// Every changed line of one hunk.
pub fn hunk_lines(diff: &FileDiff, hunk: usize) -> Lines {
    let lines = diff.hunks.get(hunk).map_or(&[][..], |hunk| &hunk.lines);
    (0..lines.len())
        .filter(|&line| lines[line].kind != LineKind::Context)
        .map(|line| (hunk, line))
        .collect()
}

/// The patch that carries only the selected lines of `diff`.
///
/// A forward patch is applied as it is (staging): unselected added lines are
/// dropped and unselected removed lines become context. A reverse patch is
/// applied with `--reverse` (unstaging and discarding): unselected added
/// lines become context and unselected removed lines are dropped. `None`
/// when no selected change is left.
pub fn build(diff: &FileDiff, selected: &Lines, reverse: bool) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    // Set when an unselected line of the file stays behind as context, so
    // the patch no longer creates, deletes or renames the whole file.
    let mut partial = false;
    // How far the hunks before this one shift the other side's line numbers.
    let mut shift = 0i64;

    for (h, hunk) in diff.hunks.iter().enumerate() {
        let mut lines = Vec::new();
        let (mut old_len, mut new_len, mut changes) = (0u32, 0u32, 0);
        for (l, line) in hunk.lines.iter().enumerate() {
            let chosen = selected.contains(&(h, l));
            let kind = match line.kind {
                LineKind::Context => LineKind::Context,
                kind if chosen => kind,
                // Staying as it is on the side the patch is applied to.
                LineKind::Remove if !reverse => LineKind::Context,
                LineKind::Add if reverse => LineKind::Context,
                _ => continue,
            };
            partial |= kind == LineKind::Context && line.kind != LineKind::Context;
            let sign = match kind {
                LineKind::Context => b' ',
                LineKind::Add => b'+',
                LineKind::Remove => b'-',
            };
            if kind != LineKind::Add {
                old_len += 1;
            }
            if kind != LineKind::Remove {
                new_len += 1;
            }
            if kind != LineKind::Context {
                changes += 1;
            }
            lines.push(sign);
            lines.extend_from_slice(&line.text);
            if !line.text.ends_with(b"\n") {
                lines.extend_from_slice(b"\n\\ No newline at end of file\n");
            }
        }
        if changes == 0 {
            continue;
        }
        // A side with no lines names the line before it, so compare the
        // positions as if both sides had one.
        let first = |start: u32, len: u32| i64::from(start) + i64::from(len == 0);
        let (old_first, new_first) = if reverse {
            let new_first = first(hunk.new_start, hunk.new_len);
            (new_first - shift, new_first)
        } else {
            let old_first = first(hunk.old_start, hunk.old_len);
            (old_first, old_first + shift)
        };
        shift += i64::from(new_len) - i64::from(old_len);
        let start = |first: i64, len: u32| (first - i64::from(len == 0)).max(0);
        body.extend_from_slice(
            format!(
                "@@ -{},{old_len} +{},{new_len} @@\n",
                start(old_first, old_len),
                start(new_first, new_len),
            )
            .as_bytes(),
        );
        body.extend_from_slice(&lines);
    }
    if body.is_empty() {
        return None;
    }

    let has = |needle: &[u8]| diff.header.windows(needle.len()).any(|w| w == needle);
    // The side the patch is applied to keeps its file, under the name it
    // has there.
    let path = if reverse {
        diff.new_path.as_ref()
    } else {
        diff.old_path.as_ref()
    };
    let renamed = has(b"\nrename from ");
    let mut patch = match path {
        Some(path) if partial || renamed => {
            let mut header = b"diff --git ".to_vec();
            header.extend_from_slice(&quote_path(b"a/", &path.0));
            header.push(b' ');
            header.extend_from_slice(&quote_path(b"b/", &path.0));
            header.extend_from_slice(b"\n--- ");
            header.extend_from_slice(&quote_path(b"a/", &path.0));
            header.extend_from_slice(b"\n+++ ");
            header.extend_from_slice(&quote_path(b"b/", &path.0));
            header.push(b'\n');
            header
        }
        // The blob IDs on the "index" line no longer describe this patch.
        _ => diff
            .header
            .split_inclusive(|&b| b == b'\n')
            .filter(|line| !line.starts_with(b"index "))
            .flatten()
            .copied()
            .collect(),
    };
    patch.extend_from_slice(&body);
    Some(patch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::diff::parse_file_diff;

    fn text(patch: Option<Vec<u8>>) -> String {
        String::from_utf8(patch.unwrap()).unwrap()
    }

    const MIXED: &[u8] = b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n\
@@ -1,4 +1,5 @@\n one\n-two\n+TWO\n+2.5\n three\n four\n\
@@ -10,2 +11,3 @@\n ten\n+new\n eleven\n";

    fn mixed() -> FileDiff {
        let mut diff = parse_file_diff(MIXED);
        diff.old_path = Some("f".into());
        diff.new_path = Some("f".into());
        diff
    }

    #[test]
    fn forward_drops_unselected_adds_and_keeps_unselected_removes() {
        // Only "+2.5" of the first hunk.
        let patch = text(build(&mixed(), &Lines::from([(0, 3)]), false));
        assert_eq!(
            patch,
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,4 +1,5 @@\n one\n two\n+2.5\n three\n four\n"
        );
    }

    #[test]
    fn reverse_keeps_unselected_adds_and_drops_unselected_removes() {
        let patch = text(build(&mixed(), &Lines::from([(0, 3)]), true));
        assert_eq!(
            patch,
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,4 +1,5 @@\n one\n TWO\n+2.5\n three\n four\n"
        );
    }

    #[test]
    fn a_later_hunk_moves_by_what_the_patch_leaves_out() {
        // The whole second hunk alone: forward keeps the old position.
        let diff = mixed();
        let second = hunk_lines(&diff, 1);
        assert!(text(build(&diff, &second, false)).contains("@@ -10,2 +10,3 @@"));
        // Reverse keeps the new position.
        assert!(text(build(&diff, &second, true)).contains("@@ -11,2 +11,3 @@"));
        // With the first hunk in, the original numbers come back.
        let mut all = hunk_lines(&diff, 0);
        all.extend(second);
        let patch = text(build(&diff, &all, false));
        assert!(patch.contains("@@ -1,4 +1,5 @@") && patch.contains("@@ -10,2 +11,3 @@"));
    }

    #[test]
    fn nothing_selected_gives_no_patch() {
        assert_eq!(build(&mixed(), &Lines::new(), false), None);
        // A context line is not a change.
        assert_eq!(build(&mixed(), &Lines::from([(0, 0)]), false), None);
    }

    #[test]
    fn part_of_a_new_file_keeps_its_header_forward_and_loses_it_in_reverse() {
        let mut diff = parse_file_diff(
            b"diff --git a/n b/n\nnew file mode 100644\nindex 0..1\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,3 @@\n+a\n+b\n+c\n",
        );
        diff.new_path = Some("n".into());
        let middle = Lines::from([(0, 1)]);
        assert_eq!(
            text(build(&diff, &middle, false)),
            "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,1 @@\n+b\n"
        );
        assert_eq!(
            text(build(&diff, &middle, true)),
            "diff --git a/n b/n\n--- a/n\n+++ b/n\n@@ -1,2 +1,3 @@\n a\n+b\n c\n"
        );
    }

    #[test]
    fn a_line_without_a_newline_keeps_its_marker() {
        let mut diff = parse_file_diff(
            b"--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-a\n+A\n end\n\\ No newline at end of file\n",
        );
        diff.old_path = Some("f".into());
        diff.new_path = Some("f".into());
        let patch = text(build(&diff, &hunk_lines(&diff, 0), false));
        assert!(patch.ends_with(" end\n\\ No newline at end of file\n"));
    }
}
