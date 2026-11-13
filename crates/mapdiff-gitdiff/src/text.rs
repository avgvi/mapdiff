//! Line and word diff between two versions of one scope's text.

use mapdiff_core::LineCount;
use similar::{Algorithm, ChangeTag, DiffTag as DiffOpTag, TextDiff, capture_diff_slices};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    Equal,
    Delete,
    Insert,
}

/// A run of text inside a line. `changed` marks the words that differ from
/// the line's counterpart on the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub changed: bool,
    pub text: String,
}

/// One line of a scope diff. Line numbers are 0-based within the scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub segments: Vec<Segment>,
}

/// Diffs two texts line by line. Where a run of lines was replaced by
/// another, lines are compared pairwise and the words that differ are marked.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let plain = |text: &str| -> Vec<Segment> {
        if text.is_empty() {
            Vec::new()
        } else {
            vec![Segment {
                changed: false,
                text: text.to_owned(),
            }]
        }
    };

    let mut lines = Vec::new();
    for operation in capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines) {
        let (tag, old_range, new_range) = operation.as_tag_tuple();
        match tag {
            DiffOpTag::Equal => {
                for (old_index, new_index) in old_range.zip(new_range) {
                    lines.push(DiffLine {
                        tag: DiffTag::Equal,
                        old_line: Some(old_index as u32),
                        new_line: Some(new_index as u32),
                        segments: plain(new_lines[new_index]),
                    });
                }
            }
            DiffOpTag::Delete | DiffOpTag::Insert | DiffOpTag::Replace => {
                // The n-th old line of the run is compared with the n-th new one.
                let marked: Vec<Option<(Vec<Segment>, Vec<Segment>)>> = old_range
                    .clone()
                    .zip(new_range.clone())
                    .map(|(old_index, new_index)| {
                        mark_words(old_lines[old_index], new_lines[new_index])
                    })
                    .collect();
                for (offset, old_index) in old_range.clone().enumerate() {
                    let segments = match marked.get(offset) {
                        Some(Some((old_segments, _))) => old_segments.clone(),
                        _ => plain(old_lines[old_index]),
                    };
                    lines.push(DiffLine {
                        tag: DiffTag::Delete,
                        old_line: Some(old_index as u32),
                        new_line: None,
                        segments,
                    });
                }
                for (offset, new_index) in new_range.enumerate() {
                    let segments = match marked.get(offset) {
                        Some(Some((_, new_segments))) => new_segments.clone(),
                        _ => plain(new_lines[new_index]),
                    };
                    lines.push(DiffLine {
                        tag: DiffTag::Insert,
                        old_line: None,
                        new_line: Some(new_index as u32),
                        segments,
                    });
                }
            }
        }
    }
    lines
}

/// Lines that share less than this much of their text are too different for
/// word marks to help; they are shown as plainly replaced.
const MIN_SHARED: f64 = 0.4;

/// The two lines cut into segments, with the differing words marked.
/// `None` when the lines have too little in common.
fn mark_words(old: &str, new: &str) -> Option<(Vec<Segment>, Vec<Segment>)> {
    let (old_words, new_words) = (words(old), words(new));
    let operations = capture_diff_slices(Algorithm::Myers, &old_words, &new_words);

    let mut shared = 0usize;
    let (mut old_segments, mut new_segments) = (Vec::new(), Vec::new());
    for operation in operations {
        let (tag, old_range, new_range) = operation.as_tag_tuple();
        let old_text: String = old_words[old_range].concat();
        let new_text: String = new_words[new_range].concat();
        let changed = tag != DiffOpTag::Equal;
        if !changed {
            shared += old_text.trim().len();
        }
        push_segment(&mut old_segments, old_text, changed);
        push_segment(&mut new_segments, new_text, changed);
    }

    let longest = old.trim().len().max(new.trim().len());
    if longest == 0 || (shared as f64) < MIN_SHARED * longest as f64 {
        return None;
    }
    Some((old_segments, new_segments))
}

fn push_segment(segments: &mut Vec<Segment>, text: String, changed: bool) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some(last) if last.changed == changed => last.text.push_str(&text),
        _ => segments.push(Segment { changed, text }),
    }
}

/// Splits a line into names and numbers, runs of spaces, and single
/// punctuation characters, so a mark covers just the word that changed.
fn words(line: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };

    let mut out = Vec::new();
    let mut start = 0;
    let mut previous: Option<Class> = None;
    for (position, character) in line.char_indices() {
        let current = class(character);
        // Punctuation never joins its neighbours, even the same character.
        let boundary =
            previous.is_some_and(|previous| previous != current || current == Class::Other);
        if boundary {
            out.push(&line[start..position]);
            start = position;
        }
        previous = Some(current);
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

/// How many lines differ between two texts. With `ignore_whitespace`, lines
/// that differ only in spacing count as equal.
pub fn line_delta(old: &[u8], new: &[u8], ignore_whitespace: bool) -> LineCount {
    let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    let normalise = |text: &str| -> String {
        if !ignore_whitespace {
            return text.to_owned();
        }
        text.lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (old, new) = (normalise(&old), normalise(&new));
    let diff = TextDiff::from_lines(&old, &new);
    let mut count = LineCount::default();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => count.added += 1,
            ChangeTag::Delete => count.removed += 1,
            ChangeTag::Equal => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edited_line_marks_only_the_changed_words() {
        let lines = diff_lines(
            "let total = a + b;\nreturn total;\n",
            "let total = a - b;\nreturn total;\n",
        );
        let deleted: Vec<_> = lines
            .iter()
            .filter(|line| line.tag == DiffTag::Delete)
            .collect();
        let inserted: Vec<_> = lines
            .iter()
            .filter(|line| line.tag == DiffTag::Insert)
            .collect();
        assert_eq!((deleted.len(), inserted.len()), (1, 1));
        let changed: Vec<&str> = inserted[0]
            .segments
            .iter()
            .filter(|segment| segment.changed)
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(changed, ["-"]);
        assert!(lines.iter().any(|line| line.tag == DiffTag::Equal));
    }

    #[test]
    fn a_renamed_call_marks_only_the_name() {
        let lines = diff_lines(
            "def parse_links(value: str):\n",
            "def _parse_links(value: str):\n",
        );
        let changed = |tag: DiffTag| -> Vec<String> {
            lines
                .iter()
                .filter(|line| line.tag == tag)
                .flat_map(|line| line.segments.iter())
                .filter(|segment| segment.changed)
                .map(|segment| segment.text.clone())
                .collect()
        };
        assert_eq!(changed(DiffTag::Delete), ["parse_links"]);
        assert_eq!(changed(DiffTag::Insert), ["_parse_links"]);
    }

    #[test]
    fn unrelated_lines_get_no_word_marks() {
        let lines = diff_lines("return total / count\n", "raise ValueError(message)\n");
        assert!(
            lines
                .iter()
                .all(|line| line.segments.iter().all(|segment| !segment.changed))
        );
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn line_numbers_count_from_zero_on_each_side() {
        let lines = diff_lines("a\nb\nc\n", "a\nc\nd\n");
        let summary: Vec<_> = lines
            .iter()
            .map(|line| (line.tag, line.old_line, line.new_line))
            .collect();
        assert_eq!(
            summary,
            [
                (DiffTag::Equal, Some(0), Some(0)),
                (DiffTag::Delete, Some(1), None),
                (DiffTag::Equal, Some(2), Some(1)),
                (DiffTag::Insert, None, Some(2)),
            ]
        );
    }

    #[test]
    fn spacing_only_differences_can_be_ignored() {
        let (old, new) = (b"a  +  b\nc\n", b"a + b\nd\n");
        assert_eq!(
            line_delta(old, new, false),
            LineCount {
                added: 2,
                removed: 2
            }
        );
        assert_eq!(
            line_delta(old, new, true),
            LineCount {
                added: 1,
                removed: 1
            }
        );
    }
}
