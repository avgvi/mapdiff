//! What a language collector reads off one file's syntax: names that are
//! used, names that imports bind, and the base classes of each class.
//! Nothing here is resolved yet; `resolve` turns these into references.

use mapdiff_core::RefKind;
use tree_sitter::Node;

/// The first segment of a path that starts at the object a method is called
/// on: `self.x`, `this.x`.
pub const SELF: &str = "<self>";
/// The first segment of a path that starts at the enclosing type: `Self::x`.
pub const SELF_TYPE: &str = "<Self>";
/// The first segment of a path that starts at the base class: `super().x`.
pub const SUPER: &str = "<super>";

/// One name in a dotted path, with where it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub name: String,
    pub start: u32,
    pub end: u32,
}

impl Seg {
    pub fn of(node: Node<'_>, source: &[u8]) -> Self {
        Self {
            name: String::from_utf8_lossy(&source[node.byte_range()]).into_owned(),
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }
    }

    /// A segment that stands for a keyword such as `self`, at a node.
    pub fn marker(name: &str, node: Node<'_>) -> Self {
        Self {
            name: name.to_owned(),
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }
    }
}

/// A name, or a dotted path of names, used somewhere in the file. Names
/// bound locally (parameters, local variables) are already left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub segments: Vec<Seg>,
    pub kind: RefKind,
}

/// What an import statement points at, in the language's own terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A Rust path as written: `crate::shapes::Circle`.
    Rust { path: Vec<String> },
    /// `from <level dots><module> import <name>`. No name means the module.
    Python {
        level: u32,
        module: Vec<String>,
        name: Option<String>,
    },
    /// A JavaScript or TypeScript module specifier and the export taken from
    /// it. No name means the whole module; `default` is the default export.
    Ecma {
        specifier: String,
        name: Option<String>,
    },
}

/// A name that an import (or re-export) statement makes available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The name bound in this file. `None` for an import that binds nothing
    /// but still depends on its target, such as `import "./setup"`.
    pub local: Option<String>,
    /// `import *`, `use a::*`, `export * from`: every name of the target.
    pub star: bool,
    /// `export ... from`, which passes a name on without binding it here.
    pub reexport: bool,
    pub target: Target,
    /// Where the statement starts, to find the scope it sits in.
    pub statement: u32,
    /// Where the bound name is written, for the import's own reference.
    pub start: u32,
    pub end: u32,
}

/// The base classes of one class, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bases {
    /// A position inside the class, to find its scope.
    pub class: u32,
    pub paths: Vec<Vec<Seg>>,
}

#[derive(Debug, Default)]
pub struct Facts {
    pub uses: Vec<Use>,
    pub bindings: Vec<Binding>,
    pub bases: Vec<Bases>,
}

impl Facts {
    pub fn add_use(&mut self, segments: Vec<Seg>, kind: RefKind) {
        if !segments.is_empty() {
            self.uses.push(Use { segments, kind });
        }
    }
}

/// The names bound inside each enclosing function, innermost last. A use
/// whose first name is in any of them is local and is not recorded.
#[derive(Default)]
pub struct Locals {
    frames: Vec<std::collections::HashSet<String>>,
}

impl Locals {
    pub fn push(&mut self, names: std::collections::HashSet<String>) {
        self.frames.push(names);
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn contains(&self, name: &str) -> bool {
        self.frames.iter().any(|frame| frame.contains(name))
    }
}

pub fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).unwrap_or("")
}

/// The contents of a string literal node without its quotes, when it is a
/// plain literal.
pub fn string_value(node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = text(node, source);
    let trimmed = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    (!trimmed.is_empty() && trimmed.len() + 2 == raw.len() && !trimmed.contains("${"))
        .then(|| trimmed.to_owned())
}
