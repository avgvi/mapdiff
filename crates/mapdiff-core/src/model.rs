//! The scope model.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{Access, SCHEMA_VERSION, ScopeKind};

/// A point in a file. Both numbers are 0-based; `column` counts UTF-8 bytes
/// from the start of the line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// A half-open span of a file's UTF-8 bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Range {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn len(&self) -> u32 {
        self.end_byte - self.start_byte
    }

    pub fn is_empty(&self) -> bool {
        self.end_byte == self.start_byte
    }

    pub fn contains(&self, other: &Range) -> bool {
        self.start_byte <= other.start_byte && other.end_byte <= self.end_byte
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Python,
    Typescript,
    Javascript,
    /// Directories and non-code files.
    None,
}

/// Flags on a scope. The set is closed so the UI can rely on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Modifier {
    Abstract,
    Async,
    Const,
    Declare,
    DefaultExport,
    Error,
    Generator,
    Override,
    Readonly,
    Static,
    Test,
    Unsafe,
}

/// The structured form of an ID. Code that needs the parts of an ID reads
/// these fields; the ID string itself is never parsed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopePath {
    /// Repository-relative path with `/` separators. `.` for the root.
    pub file: String,
    /// One entry per nesting level inside the file. Empty for directories
    /// and files.
    pub segments: Vec<String>,
}

impl ScopePath {
    pub fn id(&self) -> String {
        if self.segments.is_empty() {
            return self.file.clone();
        }
        let mut id = self.file.clone();
        for segment in &self.segments {
            id.push_str("::");
            id.push_str(segment);
        }
        id
    }
}

/// Five hashes over different views of the same text. See the Hashes section
/// of the spec for what each one detects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hashes {
    pub text: String,
    pub tokens: String,
    pub code: String,
    pub signature: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub id: String,
    pub path: ScopePath,
    pub kind: ScopeKind,
    pub lang_kind: String,
    pub language: Language,
    pub name: Option<String>,
    pub access: Access,
    pub modifiers: Vec<Modifier>,
    pub range: Range,
    pub name_range: Option<Range>,
    pub header_range: Range,
    pub body_range: Option<Range>,
    pub signature: Option<String>,
    pub hashes: Hashes,
    pub parent: Option<String>,
    pub children: Vec<String>,
}

impl Scope {
    pub fn has(&self, modifier: Modifier) -> bool {
        self.modifiers.contains(&modifier)
    }
}

/// Every scope of a repository (or of one file), in pre-order: a parent
/// always comes before its children, and siblings are in source order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeTree {
    pub schema_version: u32,
    pub scopes: Vec<Scope>,
}

impl ScopeTree {
    pub fn new(scopes: Vec<Scope>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            scopes,
        }
    }

    pub fn root(&self) -> Option<&Scope> {
        self.scopes.first()
    }

    /// Scope IDs mapped to their position in `scopes`.
    pub fn index(&self) -> HashMap<&str, usize> {
        self.scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), position))
            .collect()
    }

    /// The IDs of a scope and of everything inside it. Empty when there is
    /// no scope with that ID.
    pub fn subtree_ids(&self, id: &str) -> std::collections::HashSet<String> {
        let index = self.index();
        let mut ids = std::collections::HashSet::new();
        let mut pending: Vec<&str> = index.contains_key(id).then_some(id).into_iter().collect();
        while let Some(current) = pending.pop() {
            if ids.insert(current.to_owned())
                && let Some(position) = index.get(current)
            {
                pending.extend(self.scopes[*position].children.iter().map(String::as_str));
            }
        }
        ids
    }

    pub fn get(&self, id: &str) -> Option<&Scope> {
        self.scopes.iter().find(|scope| scope.id == id)
    }

    /// Checks the structural rules of the model and returns one message per
    /// broken rule. An empty list means the tree is sound.
    ///
    /// The rules: IDs are unique and match their paths; parents come before
    /// children; inside a file, children lie within their parent, in source
    /// order, without overlapping (the coverage rule of the spec).
    pub fn violations(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let index = self.index();
        if index.len() != self.scopes.len() {
            let mut seen = std::collections::HashSet::new();
            for scope in &self.scopes {
                if !seen.insert(scope.id.as_str()) {
                    problems.push(format!("duplicate ID {}", scope.id));
                }
            }
        }

        for (position, scope) in self.scopes.iter().enumerate() {
            let id = &scope.id;
            if scope.path.id() != *id {
                problems.push(format!("{id}: ID does not match its path"));
            }
            match &scope.parent {
                None if position != 0 => {
                    problems.push(format!("{id}: only the root may lack a parent"))
                }
                None => {}
                Some(parent) => match index.get(parent.as_str()) {
                    None => problems.push(format!("{id}: parent {parent} does not exist")),
                    Some(parent_position) if *parent_position >= position => {
                        problems.push(format!("{id}: appears before its parent"));
                    }
                    Some(parent_position) => {
                        if !self.scopes[*parent_position].children.contains(id) {
                            problems.push(format!("{id}: missing from its parent's children"));
                        }
                    }
                },
            }

            let range = &scope.range;
            if scope.header_range.start_byte != range.start_byte
                || !range.contains(&scope.header_range)
            {
                problems.push(format!("{id}: header is not at the start of the scope"));
            }
            if scope.body_range.is_some_and(|body| !range.contains(&body)) {
                problems.push(format!("{id}: body lies outside the scope"));
            }
            if scope.name_range.is_some_and(|name| !range.contains(&name)) {
                problems.push(format!("{id}: name lies outside the scope"));
            }

            let in_file = scope.kind != ScopeKind::Directory;
            let mut previous_position = position;
            let mut previous_end = range.start_byte;
            for child_id in &scope.children {
                let Some(child_position) = index.get(child_id.as_str()).copied() else {
                    problems.push(format!("{id}: child {child_id} does not exist"));
                    continue;
                };
                let child = &self.scopes[child_position];
                if child.parent.as_deref() != Some(id.as_str()) {
                    problems.push(format!(
                        "{child_id}: listed under {id} but names another parent"
                    ));
                }
                if child_position <= previous_position {
                    problems.push(format!("{child_id}: out of order among its siblings"));
                }
                previous_position = child_position;
                if in_file {
                    if !range.contains(&child.range) {
                        problems.push(format!("{child_id}: lies outside its parent"));
                    }
                    if child.range.start_byte < previous_end {
                        problems.push(format!("{child_id}: overlaps the sibling before it"));
                    }
                    if child.range.is_empty() {
                        problems.push(format!("{child_id}: is empty"));
                    }
                    previous_end = child.range.end_byte;
                }
            }
        }
        problems
    }

    /// An indented, one-line-per-scope rendering for people and for snapshot
    /// tests. Lines are 1-based here because that is what editors show.
    pub fn render_text(&self) -> String {
        let index = self.index();
        let mut out = String::new();
        if let Some(root) = self.root() {
            self.render_scope(root, 0, &index, &mut out);
        }
        out
    }

    fn render_scope(
        &self,
        scope: &Scope,
        depth: usize,
        index: &HashMap<&str, usize>,
        out: &mut String,
    ) {
        let segment = scope
            .path
            .segments
            .last()
            .map(String::as_str)
            .unwrap_or(scope.path.file.as_str());
        let _ = write!(
            out,
            "{}{} [{}",
            "  ".repeat(depth),
            segment,
            scope.lang_kind
        );
        if scope.lang_kind != kind_name(scope.kind) {
            let _ = write!(out, " -> {}", kind_name(scope.kind));
        }
        out.push(']');
        if !matches!(
            scope.kind,
            ScopeKind::Directory | ScopeKind::File | ScopeKind::Document
        ) {
            let _ = write!(out, " {}", access_name(scope.access));
        }
        for modifier in &scope.modifiers {
            let _ = write!(out, " +{}", modifier_name(*modifier));
        }
        if scope.kind != ScopeKind::Directory {
            let _ = write!(
                out,
                " L{}-{}",
                scope.range.start.line + 1,
                end_line(&scope.range) + 1
            );
        }
        out.push('\n');
        for child in &scope.children {
            if let Some(position) = index.get(child.as_str()) {
                self.render_scope(&self.scopes[*position], depth + 1, index, out);
            }
        }
    }
}

/// The last line a range touches. A range that ends at column 0 ends on the
/// line before.
fn end_line(range: &Range) -> u32 {
    if range.end.column == 0 && range.end.line > range.start.line {
        range.end.line - 1
    } else {
        range.end.line
    }
}

pub fn kind_name(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Directory => "directory",
        ScopeKind::File => "file",
        ScopeKind::Document => "document",
        ScopeKind::Module => "module",
        ScopeKind::Class => "class",
        ScopeKind::Interface => "interface",
        ScopeKind::Enum => "enum",
        ScopeKind::Variant => "variant",
        ScopeKind::TypeAlias => "type_alias",
        ScopeKind::Impl => "impl",
        ScopeKind::Function => "function",
        ScopeKind::Method => "method",
        ScopeKind::Constructor => "constructor",
        ScopeKind::Field => "field",
        ScopeKind::Constant => "constant",
        ScopeKind::Macro => "macro",
        ScopeKind::Import => "import",
        ScopeKind::Block => "block",
    }
}

pub fn access_name(access: Access) -> &'static str {
    match access {
        Access::Private => "private",
        Access::Restricted => "restricted",
        Access::Public => "public",
    }
}

pub fn modifier_name(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Abstract => "abstract",
        Modifier::Async => "async",
        Modifier::Const => "const",
        Modifier::Declare => "declare",
        Modifier::DefaultExport => "default-export",
        Modifier::Error => "error",
        Modifier::Generator => "generator",
        Modifier::Override => "override",
        Modifier::Readonly => "readonly",
        Modifier::Static => "static",
        Modifier::Test => "test",
        Modifier::Unsafe => "unsafe",
    }
}
