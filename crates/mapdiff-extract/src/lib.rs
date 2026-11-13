//! Parses source files into scope trees.

mod build;
mod ecma;
pub mod highlight;
pub mod parse;
mod python;
mod rust;
pub mod tests_rule;
mod walk;

use std::path::Path;

use mapdiff_core::{Language, Modifier, Scope, ScopeKind};
use tree_sitter::Node;

pub use parse::{Grammar, dump_tree};
pub use tests_rule::is_test_path;
pub use walk::{Extracted, extract_repo, extract_repo_parts, extract_repo_with_tests, stamp};

type Predicate = fn(Node<'_>) -> bool;

/// Extracts the scopes of one file. `path` is the repository-relative path
/// with `/` separators; it becomes the file's ID.
///
/// The result is in pre-order with the file scope first. A file that is not
/// source code, is not UTF-8, or does not parse at all becomes a single
/// `document` scope.
pub fn extract_file(path: &str, source: &[u8]) -> Vec<Scope> {
    let Some(grammar) = Grammar::for_path(Path::new(path)) else {
        return build::document(path, source, Language::None, Vec::new());
    };
    if is_listed_only(path, source) {
        return build::document(path, source, Language::None, Vec::new());
    }
    let language = grammar.language();
    if std::str::from_utf8(source).is_err() {
        return build::document(path, source, language, vec![Modifier::Error]);
    }
    let Some(tree) = parse::parse(grammar, source) else {
        return build::document(path, source, language, vec![Modifier::Error]);
    };
    let root = tree.root_node();
    if root.is_error() {
        return build::document(path, source, language, vec![Modifier::Error]);
    }

    let (is_comment, is_attachable): (Predicate, Predicate) = match grammar {
        Grammar::Rust => (rust::is_comment, rust::is_attachable),
        Grammar::Python => (python::is_comment, python::is_attachable),
        Grammar::TypeScript | Grammar::Tsx | Grammar::JavaScript => {
            (ecma::is_comment, ecma::is_attachable)
        }
    };
    let children = match grammar {
        Grammar::Rust => rust::file_items(root, source),
        Grammar::Python => python::file_items(root, source),
        Grammar::TypeScript | Grammar::Tsx | Grammar::JavaScript => ecma::file_items(root, source),
    };
    let is_script = matches!(language, Language::Typescript | Language::Javascript);
    let modifiers = if is_script && ecma::is_test_path(path) {
        vec![Modifier::Test]
    } else {
        Vec::new()
    };

    build::Builder::new(
        path,
        source,
        language,
        Some(root),
        is_comment,
        is_attachable,
    )
    .file(ScopeKind::File, modifiers, root.has_error(), children)
}

/// Source files larger than this are listed but not parsed.
const MAX_PARSED_BYTES: usize = 1024 * 1024;

/// Source files that are listed but not parsed: minified bundles, files a
/// tool generated, and very large files. Nobody reviews these by construct.
/// They become a single `document` scope.
fn is_listed_only(path: &str, source: &[u8]) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if source.len() > MAX_PARSED_BYTES || name.contains(".min.") {
        return true;
    }
    let head = String::from_utf8_lossy(&source[..source.len().min(512)]);
    head.contains("@generated") || head.contains("DO NOT EDIT")
}
