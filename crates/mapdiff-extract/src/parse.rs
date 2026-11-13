//! Language detection and tree-sitter parsing.

use std::fmt::Write as _;
use std::path::Path;

use mapdiff_core::Language;
use tree_sitter::{Node, Parser, Tree};

/// Which grammar to parse a file with. TSX and JSX need their own grammars
/// but report the same `Language` as their plain forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
}

impl Grammar {
    /// Picks a grammar from the file extension. `None` means the file is not
    /// source code we parse; it becomes a `document` scope.
    pub fn for_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Some(match extension {
            "rs" => Grammar::Rust,
            "py" | "pyi" => Grammar::Python,
            "ts" | "mts" | "cts" => Grammar::TypeScript,
            "tsx" => Grammar::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Grammar::JavaScript,
            _ => return None,
        })
    }

    pub fn language(self) -> Language {
        match self {
            Grammar::Rust => Language::Rust,
            Grammar::Python => Language::Python,
            Grammar::TypeScript | Grammar::Tsx => Language::Typescript,
            Grammar::JavaScript => Language::Javascript,
        }
    }

    fn tree_sitter(self) -> tree_sitter::Language {
        match self {
            Grammar::Rust => tree_sitter_rust::LANGUAGE.into(),
            Grammar::Python => tree_sitter_python::LANGUAGE.into(),
            Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        }
    }
}

/// Parses `source`. Returns `None` only if tree-sitter gives up entirely,
/// which in practice means the grammar failed to load.
pub fn parse(grammar: Grammar, source: &[u8]) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&grammar.tree_sitter()).ok()?;
    parser.parse(source, None)
}

/// An indented dump of the syntax tree with field names, for working out
/// which node kinds an extractor has to handle.
pub fn dump_tree(grammar: Grammar, source: &[u8]) -> String {
    let Some(tree) = parse(grammar, source) else {
        return String::from("(parse failed)\n");
    };
    let mut out = String::new();
    dump_node(tree.root_node(), None, source, 0, &mut out);
    out
}

fn dump_node(node: Node<'_>, field: Option<&str>, source: &[u8], depth: usize, out: &mut String) {
    if !node.is_named() {
        return;
    }
    let _ = write!(out, "{}", "  ".repeat(depth));
    if let Some(field) = field {
        let _ = write!(out, "{field}: ");
    }
    let _ = write!(
        out,
        "{} [{}:{}-{}:{}]",
        node.kind(),
        node.start_position().row + 1,
        node.start_position().column,
        node.end_position().row + 1,
        node.end_position().column
    );
    if node.named_child_count() == 0 {
        let text = String::from_utf8_lossy(&source[node.byte_range()]);
        let short: String = text.chars().take(40).collect();
        let _ = write!(out, " {short:?}");
    }
    out.push('\n');

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            dump_node(cursor.node(), cursor.field_name(), source, depth + 1, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}
