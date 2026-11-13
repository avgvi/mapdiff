//! The before and after of one change, ready to draw: lines paired up by a
//! diff, each cut into spans that carry a syntax class and whether the text
//! differs from the other side.

use std::path::Path;

use mapdiff_core::{Change, Range, ScopeKind};
use mapdiff_extract::Grammar;
use mapdiff_extract::highlight::{Span, highlight};
use mapdiff_gitdiff::{Comparison, DiffTag, Repo, diff_lines};
use serde::Serialize;

/// Scopes with more lines than this on either side are sent without a diff.
/// A page cannot usefully show more, and the browser slows down long before.
const MAX_LINES: usize = 20_000;

#[derive(Debug, Serialize)]
pub struct ChangeDetail {
    pub index: usize,
    pub id: String,
    /// Where the old text is. `None` when the scope was added.
    pub old: Option<Location>,
    /// Where the new text is. `None` when the scope was removed.
    pub new: Option<Location>,
    pub rows: Vec<Row>,
    /// Set when there is nothing to show, saying why.
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Location {
    pub path: String,
    /// 1-based, as editors show it.
    pub first_line: u32,
    pub last_line: u32,
}

#[derive(Debug, Serialize)]
pub struct Row {
    /// `equal`, `delete` or `insert`.
    pub tag: &'static str,
    /// 1-based line in the old file. `None` for an inserted line.
    pub old_line: Option<u32>,
    /// 1-based line in the new file. `None` for a deleted line.
    pub new_line: Option<u32>,
    pub spans: Vec<RowSpan>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RowSpan {
    pub text: String,
    /// A syntax class from `mapdiff_extract::highlight::CLASSES`.
    pub class: Option<&'static str>,
    /// True for the words that differ from the line's counterpart.
    pub changed: bool,
}

/// One side of a change: the whole lines its scope covers.
struct Side {
    location: Location,
    text: String,
    /// Highlight spans of each of those lines.
    highlights: Vec<Vec<Span>>,
}

impl Side {
    fn load(repo: &Repo, side: &mapdiff_gitdiff::Side, path: &str, range: &Range) -> Self {
        let source = repo.read(side, path);
        let first = range.start.line as usize;
        // A range that ends at column 0 ends on the line before.
        let last = if range.end.column == 0 && range.end.line > range.start.line {
            range.end.line - 1
        } else {
            range.end.line
        } as usize;

        let lines: Vec<&[u8]> = source.split(|byte| *byte == b'\n').collect();
        let last = last.min(lines.len().saturating_sub(1));
        let text = lines
            .get(first..=last)
            .unwrap_or(&[])
            .iter()
            .map(|line| {
                String::from_utf8_lossy(line)
                    .trim_end_matches('\r')
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n");

        let highlights = match Grammar::for_path(Path::new(path)) {
            Some(grammar) if std::str::from_utf8(&source).is_ok() => highlight(grammar, &source)
                .into_iter()
                .skip(first)
                .take(last + 1 - first.min(last + 1))
                .collect(),
            _ => Vec::new(),
        };

        Self {
            location: Location {
                path: path.to_owned(),
                first_line: first as u32 + 1,
                last_line: last as u32 + 1,
            },
            text,
            highlights,
        }
    }
}

pub fn change_detail(
    repo: &Repo,
    comparison: &Comparison,
    change: &Change,
    index: usize,
) -> ChangeDetail {
    let load = |side: &mapdiff_gitdiff::Side, path: &Option<String>, range: &Option<Range>| {
        let (path, range) = (path.as_deref()?, range.as_ref()?);
        Some(Side::load(repo, side, path, range))
    };
    let (old, new) = if change.kind == ScopeKind::Directory {
        (None, None)
    } else {
        (
            load(&comparison.base_side(), &change.old_path, &change.old_range),
            load(comparison.head_side(), &change.new_path, &change.new_range),
        )
    };

    let empty = String::new();
    let old_text = old.as_ref().map_or(&empty, |side| &side.text);
    let new_text = new.as_ref().map_or(&empty, |side| &side.text);
    let lines = old_text.lines().count().max(new_text.lines().count());

    let (rows, note) = if change.kind == ScopeKind::Directory {
        (
            Vec::new(),
            Some(String::from(
                "A directory. Pick a file or a scope inside it.",
            )),
        )
    } else if old_text.contains('\0') || new_text.contains('\0') {
        (
            Vec::new(),
            Some(String::from("A binary file. There is no text to compare.")),
        )
    } else if lines > MAX_LINES {
        (
            Vec::new(),
            Some(format!(
                "{lines} lines is too many to show at once. Pick a scope inside it."
            )),
        )
    } else if old_text.is_empty() && new_text.is_empty() {
        (Vec::new(), Some(String::from("No text on either side.")))
    } else {
        (rows(old.as_ref(), new.as_ref(), old_text, new_text), None)
    };

    ChangeDetail {
        index,
        id: change.id().to_owned(),
        old: old.map(|side| side.location),
        new: new.map(|side| side.location),
        rows,
        note,
    }
}

fn rows(old: Option<&Side>, new: Option<&Side>, old_text: &str, new_text: &str) -> Vec<Row> {
    let old_first = old.map_or(1, |side| side.location.first_line);
    let new_first = new.map_or(1, |side| side.location.first_line);
    let no_highlights: Vec<Span> = Vec::new();

    diff_lines(old_text, new_text)
        .into_iter()
        .map(|line| {
            // Syntax classes come from the side the line is shown on.
            let highlights = match line.tag {
                DiffTag::Delete => line
                    .old_line
                    .and_then(|relative| old?.highlights.get(relative as usize)),
                DiffTag::Equal | DiffTag::Insert => line
                    .new_line
                    .and_then(|relative| new?.highlights.get(relative as usize)),
            }
            .unwrap_or(&no_highlights);

            let mut text = String::new();
            let mut changed = Vec::new();
            for segment in &line.segments {
                let start = text.len();
                text.push_str(&segment.text);
                if segment.changed {
                    changed.push((start, text.len()));
                }
            }

            Row {
                tag: match line.tag {
                    DiffTag::Equal => "equal",
                    DiffTag::Delete => "delete",
                    DiffTag::Insert => "insert",
                },
                old_line: line.old_line.map(|relative| old_first + relative),
                new_line: line.new_line.map(|relative| new_first + relative),
                spans: spans(&text, highlights, &changed),
            }
        })
        .collect()
}

/// Cuts a line wherever its syntax class or its changed state switches.
pub(crate) fn spans(text: &str, highlights: &[Span], changed: &[(usize, usize)]) -> Vec<RowSpan> {
    let mut cuts = vec![0, text.len()];
    for span in highlights {
        cuts.extend([span.start as usize, span.end as usize]);
    }
    for (start, end) in changed {
        cuts.extend([*start, *end]);
    }
    cuts.retain(|cut| *cut <= text.len() && text.is_char_boundary(*cut));
    cuts.sort_unstable();
    cuts.dedup();

    let mut out: Vec<RowSpan> = Vec::new();
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let class = highlights
            .iter()
            .find(|span| span.start as usize <= start && end <= span.end as usize)
            .map(|span| span.class);
        let is_changed = changed
            .iter()
            .any(|(from, to)| *from <= start && end <= *to);
        match out.last_mut() {
            Some(last) if last.class == class && last.changed == is_changed => {
                last.text.push_str(&text[start..end]);
            }
            _ => out.push(RowSpan {
                text: text[start..end].to_owned(),
                class,
                changed: is_changed,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_cut_where_class_or_changed_state_switches() {
        let highlights = [
            Span {
                start: 0,
                end: 3,
                class: "keyword",
            },
            Span {
                start: 4,
                end: 9,
                class: "variable",
            },
        ];
        let out = spans("let total = 1;", &highlights, &[(4, 9)]);
        let simple: Vec<(&str, Option<&str>, bool)> = out
            .iter()
            .map(|span| (span.text.as_str(), span.class, span.changed))
            .collect();
        assert_eq!(
            simple,
            [
                ("let", Some("keyword"), false),
                (" ", None, false),
                ("total", Some("variable"), true),
                (" = 1;", None, false),
            ]
        );
    }

    #[test]
    fn spans_never_split_a_character() {
        // A highlight column that lands inside a multi-byte character is ignored.
        let out = spans(
            "é = 1",
            &[Span {
                start: 1,
                end: 2,
                class: "variable",
            }],
            &[],
        );
        let joined: String = out.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(joined, "é = 1");
    }
}
