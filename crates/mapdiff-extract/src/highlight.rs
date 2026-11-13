//! Syntax highlighting, using the same grammars as the extractors.

use std::sync::OnceLock;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::parse::Grammar;

/// The classes a span can have. Grammar queries use dotted names such as
/// `function.method`; each is reduced to the longest of these it starts with.
pub const CLASSES: [&str; 16] = [
    "attribute",
    "comment",
    "constant",
    "constructor",
    "embedded",
    "function",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "tag",
    "type",
    "variable",
];

/// A highlighted stretch of one line. Columns count UTF-8 bytes from the
/// start of the line; the end is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub class: &'static str,
}

/// Highlights a whole file and returns the spans of each line, in order.
/// Text outside every span has no class. Returns one empty list per line
/// when the file cannot be highlighted.
pub fn highlight(grammar: Grammar, source: &[u8]) -> Vec<Vec<Span>> {
    let mut line_starts = vec![0usize];
    for (offset, byte) in source.iter().enumerate() {
        if *byte == b'\n' {
            line_starts.push(offset + 1);
        }
    }
    let mut lines: Vec<Vec<Span>> = vec![Vec::new(); line_starts.len()];

    let Some(config) = configuration(grammar) else {
        return lines;
    };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, source, None, None, |_| None) else {
        return lines;
    };

    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(highlight)) => stack.push(highlight.0),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(class) = stack.last().and_then(|index| CLASSES.get(*index)) else {
                    continue;
                };
                // A span that crosses lines (a block comment, a multi-line
                // string) is cut at each line end.
                let mut line = line_starts.partition_point(|line_start| *line_start <= start) - 1;
                let mut position = start;
                while position < end && line < line_starts.len() {
                    let line_start = line_starts[line];
                    let line_end = line_starts.get(line + 1).copied().unwrap_or(source.len());
                    let stop = end.min(line_end);
                    // Leave the newline itself out of the span.
                    let visible =
                        if stop == line_end && stop > position && source[stop - 1] == b'\n' {
                            stop - 1
                        } else {
                            stop
                        };
                    if visible > position {
                        lines[line].push(Span {
                            start: (position - line_start) as u32,
                            end: (visible - line_start) as u32,
                            class,
                        });
                    }
                    position = stop;
                    line += 1;
                }
            }
            // A failure part-way leaves the rest of the file plain.
            Err(_) => break,
        }
    }
    lines
}

/// The compiled highlight query of a grammar, built once.
fn configuration(grammar: Grammar) -> Option<&'static HighlightConfiguration> {
    static RUST: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static PYTHON: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static TYPESCRIPT: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static TSX: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static JAVASCRIPT: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();

    let build = |language: tree_sitter::Language, name: &str, query: String| {
        let mut config = HighlightConfiguration::new(language, name, &query, "", "").ok()?;
        config.configure(&CLASSES);
        Some(config)
    };
    // The TypeScript query only adds to the JavaScript one, so both are needed.
    let script = |extra: &str| {
        format!(
            "{}\n{}\n{extra}",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY
        )
    };

    match grammar {
        Grammar::Rust => RUST.get_or_init(|| {
            build(
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
            )
        }),
        Grammar::Python => PYTHON.get_or_init(|| {
            build(
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
            )
        }),
        Grammar::TypeScript => TYPESCRIPT.get_or_init(|| {
            build(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                script(""),
            )
        }),
        Grammar::Tsx => TSX.get_or_init(|| {
            build(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                script(tree_sitter_javascript::JSX_HIGHLIGHT_QUERY),
            )
        }),
        Grammar::JavaScript => JAVASCRIPT.get_or_init(|| {
            build(
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
            )
        }),
    }
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes_of(grammar: Grammar, source: &str) -> Vec<(String, &'static str)> {
        let lines: Vec<&str> = source.split('\n').collect();
        let mut found = Vec::new();
        for (line, spans) in highlight(grammar, source.as_bytes()).iter().enumerate() {
            for span in spans {
                let text = &lines[line][span.start as usize..span.end as usize];
                found.push((text.to_owned(), span.class));
            }
        }
        found
    }

    #[test]
    fn every_grammar_has_a_working_query() {
        for (grammar, source, keyword) in [
            (Grammar::Rust, "fn main() {}\n", "fn"),
            (Grammar::Python, "def main():\n    pass\n", "def"),
            (
                Grammar::TypeScript,
                "function main(): void {}\n",
                "function",
            ),
            (Grammar::Tsx, "const a = <b>text</b>;\n", "const"),
            (Grammar::JavaScript, "function main() {}\n", "function"),
        ] {
            let found = classes_of(grammar, source);
            assert!(
                found.contains(&(keyword.to_owned(), "keyword")),
                "{grammar:?}: {found:?}"
            );
        }
    }

    #[test]
    fn a_span_that_crosses_lines_is_cut_at_each_line() {
        let source = "/* one\n   two */\nfn main() {}\n";
        let lines = highlight(Grammar::Rust, source.as_bytes());
        assert_eq!(
            lines[0],
            [Span {
                start: 0,
                end: 6,
                class: "comment"
            }]
        );
        assert_eq!(
            lines[1],
            [Span {
                start: 0,
                end: 9,
                class: "comment"
            }]
        );
        assert!(lines[2].iter().any(|span| span.class == "keyword"));
    }

    #[test]
    fn one_entry_per_line_even_for_unparseable_input() {
        let lines = highlight(Grammar::Python, b"\t\t)))) def class :::\nsecond\n");
        assert_eq!(lines.len(), 3);
    }
}
