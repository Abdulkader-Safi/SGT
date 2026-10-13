use crate::model::{DiffKind, DiffLine, FileDiff, Hunk, LineKind};

/// Parses the unified diff of one file, as printed by `git diff` or
/// `git show --format=`. Paths are left empty: the caller knows them from
/// the file list, which spares reading Git's quoted path forms.
pub fn parse_file_diff(out: &[u8]) -> FileDiff {
    let mut diff = FileDiff {
        old_path: None,
        new_path: None,
        kind: DiffKind::Text,
        header: Vec::new(),
        hunks: Vec::new(),
        no_newline_old: false,
        no_newline_new: false,
    };
    let (mut old_no, mut new_no) = (0, 0);
    // How many lines of each side the current hunk still expects. A line
    // that starts with "@@" inside a hunk is content, not a new hunk.
    let (mut old_left, mut new_left) = (0u32, 0u32);

    for line in out.split_inclusive(|&b| b == b'\n') {
        let in_hunk = old_left > 0 || new_left > 0;
        if !in_hunk {
            if let Some(hunk) = parse_hunk_header(line) {
                (old_no, new_no) = (hunk.old_start, hunk.new_start);
                (old_left, new_left) = (hunk.old_len, hunk.new_len);
                diff.hunks.push(hunk);
                continue;
            }
            if line.starts_with(b"\\") {
                no_newline(&mut diff);
                continue;
            }
            if diff.hunks.is_empty() {
                if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
                    diff.kind = DiffKind::Binary {
                        old_size: 0,
                        new_size: 0,
                    };
                }
                diff.header.extend_from_slice(line);
            }
            continue;
        }

        let (kind, text) = match line.split_first() {
            Some((b'+', text)) => (LineKind::Add, text),
            Some((b'-', text)) => (LineKind::Remove, text),
            Some((b'\\', _)) => {
                // The marker can sit between the removed and added lines.
                no_newline(&mut diff);
                continue;
            }
            Some((_, text)) => (LineKind::Context, text),
            None => continue,
        };
        let line = DiffLine {
            kind,
            old_no: (kind != LineKind::Add).then_some(old_no),
            new_no: (kind != LineKind::Remove).then_some(new_no),
            text: text.to_vec(),
        };
        if kind != LineKind::Add {
            old_no += 1;
            old_left = old_left.saturating_sub(1);
        }
        if kind != LineKind::Remove {
            new_no += 1;
            new_left = new_left.saturating_sub(1);
        }
        if let Some(hunk) = diff.hunks.last_mut() {
            hunk.lines.push(line);
        }
    }

    if diff.kind == DiffKind::Text && diff.hunks.is_empty() && !diff.header.is_empty() {
        diff.kind = DiffKind::ModeOnly;
    }
    diff
}

/// Handles "\\ No newline at end of file", which describes the line before
/// it: that line loses the newline the patch format gave it.
fn no_newline(diff: &mut FileDiff) {
    let Some(last) = diff.hunks.last_mut().and_then(|hunk| hunk.lines.last_mut()) else {
        return;
    };
    if last.text.last() == Some(&b'\n') {
        last.text.pop();
    }
    diff.no_newline_old |= last.kind != LineKind::Add;
    diff.no_newline_new |= last.kind != LineKind::Remove;
}

/// Reads `@@ -12,5 +14,6 @@ fn name()`. A missing length means 1.
fn parse_hunk_header(line: &[u8]) -> Option<Hunk> {
    let line = std::str::from_utf8(line).ok()?;
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |text: &str| -> Option<(u32, u32)> {
        match text.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = range(old)?;
    let (new_start, new_len) = range(new)?;
    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
        section: section.trim().to_owned(),
        lines: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::LineKind::{Add, Context, Remove};
    use super::*;

    type Line = (LineKind, Option<u32>, Option<u32>, String);

    fn shape(diff: &FileDiff) -> Vec<Vec<Line>> {
        diff.hunks
            .iter()
            .map(|hunk| {
                hunk.lines
                    .iter()
                    .map(|l| {
                        (
                            l.kind,
                            l.old_no,
                            l.new_no,
                            String::from_utf8_lossy(&l.text).into_owned(),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn parses_hunks_with_line_numbers() {
        let out = b"diff --git a/a.txt b/a.txt\nindex 111..222 100644\n--- a/a.txt\n+++ b/a.txt\n\
@@ -1,3 +1,3 @@ fn main()\n one\n-two\n+TWO\n three\n@@ -10 +10,2 @@\n ten\n+eleven\n";
        let diff = parse_file_diff(out);
        assert_eq!(diff.kind, DiffKind::Text);
        assert!(diff.header.starts_with(b"diff --git") && diff.header.ends_with(b"+++ b/a.txt\n"));
        assert_eq!(diff.hunks[0].section, "fn main()");
        assert_eq!(
            (
                diff.hunks[1].old_start,
                diff.hunks[1].old_len,
                diff.hunks[1].new_len
            ),
            (10, 1, 2)
        );
        assert_eq!(
            shape(&diff),
            [
                vec![
                    (Context, Some(1), Some(1), "one\n".to_owned()),
                    (Remove, Some(2), None, "two\n".to_owned()),
                    (Add, None, Some(2), "TWO\n".to_owned()),
                    (Context, Some(3), Some(3), "three\n".to_owned()),
                ],
                vec![
                    (Context, Some(10), Some(10), "ten\n".to_owned()),
                    (Add, None, Some(11), "eleven\n".to_owned()),
                ],
            ]
        );
    }

    #[test]
    fn keeps_crlf_and_reads_the_no_newline_marker() {
        let out = b"--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n one\r\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n";
        let diff = parse_file_diff(out);
        assert_eq!(diff.hunks[0].lines[0].text, b"one\r\n");
        assert!(diff.no_newline_old && diff.no_newline_new);

        let only_new = parse_file_diff(
            b"--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n",
        );
        assert!(!only_new.no_newline_old && only_new.no_newline_new);
    }

    #[test]
    fn a_content_line_that_looks_like_a_hunk_header_stays_content() {
        let out = b"--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n-@@ -1 +1 @@\n+@@ -9 +9 @@\n same\n";
        let diff = parse_file_diff(out);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].lines.len(), 3);
    }

    #[test]
    fn recognises_binary_and_mode_only_diffs() {
        let binary = parse_file_diff(b"diff --git a/i.png b/i.png\nindex 1..2 100644\nBinary files a/i.png and b/i.png differ\n");
        assert!(matches!(binary.kind, DiffKind::Binary { .. }));
        let mode =
            parse_file_diff(b"diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n");
        assert_eq!(mode.kind, DiffKind::ModeOnly);
        assert_eq!(parse_file_diff(b"").kind, DiffKind::Text);
    }
}
