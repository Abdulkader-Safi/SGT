use std::ops::Range;

/// One conflicted stretch of a file, as byte ranges into its content. The
/// ranges hold whole lines and leave the marker lines out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// The whole stretch, markers included.
    pub span: Range<usize>,
    /// The current side.
    pub ours: Range<usize>,
    /// The common ancestor, when Git wrote it (diff3 style).
    pub base: Option<Range<usize>>,
    /// The incoming side.
    pub theirs: Range<usize>,
}

/// What to keep of one region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Unresolved,
    Ours,
    Theirs,
    /// The current side, then the incoming side.
    Both,
}

/// Finds the conflict regions in a file with Git's markers.
pub fn parse_conflicts(content: &[u8]) -> Vec<Region> {
    let mut regions = Vec::new();
    // (start of the stretch, start of ours, end of ours, start of base)
    let mut open: Option<(usize, usize, Option<usize>, Option<usize>)> = None;
    let mut middle: Option<usize> = None;
    let mut at = 0;
    for line in content.split_inclusive(|&b| b == b'\n') {
        let (start, end) = (at, at + line.len());
        at = end;
        let marker = |sign: u8| {
            line.len() >= 7
                && line[..7].iter().all(|&b| b == sign)
                && matches!(line.get(7), None | Some(b' ' | b'\n' | b'\r'))
        };
        match (&mut open, middle) {
            (None, _) if marker(b'<') => open = Some((start, end, None, None)),
            (Some((_, _, ours_end, base)), None) if marker(b'|') => {
                *ours_end = Some(start);
                *base = Some(end);
            }
            (Some((_, _, ours_end, _)), None) if marker(b'=') => {
                ours_end.get_or_insert(start);
                middle = Some(start);
            }
            (Some((span, ours, ours_end, base)), Some(separator)) if marker(b'>') => {
                let ours_end = ours_end.unwrap_or(separator);
                regions.push(Region {
                    span: *span..end,
                    ours: *ours..ours_end,
                    base: base.map(|base| base..separator),
                    theirs: separator
                        + content[separator..]
                            .iter()
                            .position(|&b| b == b'\n')
                            .map_or(0, |n| n + 1)..start,
                });
                open = None;
                middle = None;
            }
            _ => {}
        }
    }
    regions
}

/// The file with each region replaced by its choice. An unresolved region
/// keeps its markers.
pub fn resolve(content: &[u8], regions: &[Region], choices: &[Choice]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len());
    let mut at = 0;
    for (region, choice) in regions.iter().zip(choices) {
        out.extend_from_slice(&content[at..region.span.start]);
        match choice {
            Choice::Unresolved => out.extend_from_slice(&content[region.span.clone()]),
            Choice::Ours => out.extend_from_slice(&content[region.ours.clone()]),
            Choice::Theirs => out.extend_from_slice(&content[region.theirs.clone()]),
            Choice::Both => {
                out.extend_from_slice(&content[region.ours.clone()]);
                out.extend_from_slice(&content[region.theirs.clone()]);
            }
        }
        at = region.span.end;
    }
    out.extend_from_slice(&content[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &[u8] =
        b"top\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nmid\n\
<<<<<<< HEAD\na\n||||||| base\nold\n=======\nb\n>>>>>>> feature\nend\n";

    fn text(content: &[u8], range: &Range<usize>) -> String {
        String::from_utf8_lossy(&content[range.clone()]).into_owned()
    }

    #[test]
    fn finds_regions_with_and_without_a_base() {
        let regions = parse_conflicts(FILE);
        assert_eq!(regions.len(), 2);
        assert_eq!(text(FILE, &regions[0].ours), "ours 1\nours 2\n");
        assert_eq!(text(FILE, &regions[0].theirs), "theirs\n");
        assert_eq!(regions[0].base, None);
        assert_eq!(text(FILE, &regions[1].ours), "a\n");
        assert_eq!(text(FILE, regions[1].base.as_ref().unwrap()), "old\n");
        assert_eq!(text(FILE, &regions[1].theirs), "b\n");
        // A line of equals signs outside a conflict is content.
        assert!(parse_conflicts(b"title\n=======\ntext\n").is_empty());
    }

    #[test]
    fn resolves_each_region_by_its_choice() {
        let regions = parse_conflicts(FILE);
        let pick = |a, b| String::from_utf8(resolve(FILE, &regions, &[a, b])).unwrap();
        assert_eq!(
            pick(Choice::Ours, Choice::Theirs),
            "top\nours 1\nours 2\nmid\nb\nend\n"
        );
        assert_eq!(
            pick(Choice::Both, Choice::Ours),
            "top\nours 1\nours 2\ntheirs\nmid\na\nend\n"
        );
        let half = pick(Choice::Theirs, Choice::Unresolved);
        assert!(half.starts_with("top\ntheirs\nmid\n<<<<<<< HEAD\n"));
        assert_eq!(parse_conflicts(half.as_bytes()).len(), 1);
    }
}
