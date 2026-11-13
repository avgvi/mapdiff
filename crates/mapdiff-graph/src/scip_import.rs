//! Reads a SCIP index, as written by rust-analyzer, scip-typescript or
//! scip-python, and turns its occurrences into references between scopes.
//!
//! A SCIP index lists, per file, where each symbol is defined and where it
//! is used. A definition is attached to the scope whose name it is; a use
//! then becomes a reference from the scope around it to that scope.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use mapdiff_core::{RefKind, Reference, ScopeKind, Tier};
use mapdiff_extract::Grammar;
use protobuf::{Enum, Message};
use scip::types::{
    Document, Index as ScipIndex, Occurrence, PositionEncoding, SymbolRole, occurrence,
};

use crate::index::Index;
use crate::resolve::Resolver;
use crate::syntax_kind::kind_at;

/// What one index contributed.
#[derive(Debug, Default)]
pub struct Imported {
    pub references: Vec<Reference>,
    /// Places where the index says a name is a variable local to one
    /// function, as (file scope, byte offset). A fast-tier reference from
    /// such a place took the name for something it is not.
    pub locals: HashSet<(usize, u32)>,
    /// Places where the index points at a scope whose name is not the name
    /// written there, with the name that is written. Usually an alias, but
    /// scip-python also does this by mistake for names reached through a
    /// package that re-exports with `import *`. The merge uses it to decide
    /// which tier to believe when they disagree.
    pub renamed: HashMap<(usize, u32), String>,
    pub stats: ImportStats,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    /// Files of the index that are in the scope tree.
    pub documents: u32,
    /// Uses of a symbol, not counting definitions and local variables.
    pub uses: u32,
    /// Uses that became a reference.
    pub references: u32,
    /// Uses of something defined outside the repository.
    pub external: u32,
    /// Uses of something that is not a scope of its own: a parameter, a
    /// local variable, or the name an import statement binds.
    pub not_a_scope: u32,
}

/// One line-and-column position resolver per file.
struct Positions<'s> {
    source: &'s [u8],
    line_starts: Vec<usize>,
    encoding: PositionEncoding,
}

impl<'s> Positions<'s> {
    /// `tool` decides what an unspecified encoding means: indexers built on
    /// a language server (scip-python on pyright, scip-typescript) count
    /// UTF-16 units, as that protocol does.
    fn for_tool(source: &'s [u8], encoding: PositionEncoding, tool: &str) -> Self {
        let encoding = match encoding {
            PositionEncoding::UnspecifiedPositionEncoding if tool != "rust-analyzer" => {
                PositionEncoding::UTF16CodeUnitOffsetFromLineStart
            }
            other => other,
        };
        Self::new(source, encoding)
    }

    fn new(source: &'s [u8], encoding: PositionEncoding) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in source.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self {
            source,
            line_starts,
            encoding,
        }
    }

    /// The byte offset of a line and a character count in the index's own
    /// encoding. `None` when the position is not in the file, which means
    /// the file changed since it was indexed.
    fn byte(&self, line: i32, character: i32) -> Option<usize> {
        let start = *self.line_starts.get(usize::try_from(line).ok()?)?;
        let end = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.source.len());
        let text = std::str::from_utf8(&self.source[start..end]).ok()?;
        let wanted = usize::try_from(character).ok()?;

        let mut units = 0;
        for (offset, c) in text.char_indices() {
            if units >= wanted {
                return Some(start + offset);
            }
            units += match self.encoding {
                PositionEncoding::UTF16CodeUnitOffsetFromLineStart => c.len_utf16(),
                PositionEncoding::UTF32CodeUnitOffsetFromLineStart => 1,
                PositionEncoding::UTF8CodeUnitOffsetFromLineStart => c.len_utf8(),
                // Unspecified: `Positions::new` already chose for the tool.
                PositionEncoding::UnspecifiedPositionEncoding => c.len_utf8(),
            };
        }
        (units >= wanted).then_some(start + text.len())
    }
}

/// The range of an occurrence as (start line, start character, end line,
/// end character), from either of the two encodings the format allows.
fn range_of(occurrence: &Occurrence) -> Option<(i32, i32, i32, i32)> {
    match &occurrence.typed_range {
        Some(occurrence::Typed_range::SingleLineRange(range)) => Some((
            range.line,
            range.start_character,
            range.line,
            range.end_character,
        )),
        Some(occurrence::Typed_range::MultiLineRange(range)) => Some((
            range.start_line,
            range.start_character,
            range.end_line,
            range.end_character,
        )),
        Some(_) | None => match occurrence.range.as_slice() {
            [line, start, end] => Some((*line, *start, *line, *end)),
            [start_line, start, end_line, end] => Some((*start_line, *start, *end_line, *end)),
            _ => None,
        },
    }
}

fn is_definition(occurrence: &Occurrence) -> bool {
    occurrence.symbol_roles & SymbolRole::Definition.value() != 0
}

/// Symbols named `local N` are private to one file. Most are variables
/// inside a function, but not all: scip-typescript also names the methods of
/// an unexported object literal this way.
fn is_local(symbol: &str) -> bool {
    symbol.starts_with("local ")
}

/// Reads an index. `project` is the directory the indexer ran in, relative
/// to the repository root (`.` for the root itself); paths in the index are
/// relative to it. `fast_kinds` gives the kind the fast tier read off the
/// syntax at a place, so both tiers describe the same use the same way.
pub fn import(
    bytes: &[u8],
    project: &str,
    resolver: &Resolver<'_>,
    root: &Path,
    fast_kinds: &HashMap<(usize, u32), RefKind>,
) -> Result<Imported, String> {
    let parsed =
        ScipIndex::parse_from_bytes(bytes).map_err(|err| format!("not a SCIP index: {err}"))?;
    let index = resolver.index;
    let tool = parsed.metadata.tool_info.name.as_str();

    // Documents that are files of the scope tree, with their source.
    let mut documents: Vec<(usize, &Document, Vec<u8>)> = Vec::new();
    for document in &parsed.documents {
        let Some(path) = Index::join(project, &document.relative_path.replace('\\', "/")) else {
            continue;
        };
        let Some(file) = index.paths.get(path.as_str()).copied() else {
            continue;
        };
        if index.kind(file) != ScopeKind::File {
            continue;
        }
        let Ok(source) = std::fs::read(root.join(&path)) else {
            continue;
        };
        documents.push((file, document, source));
    }

    // First every definition, so a use can come before it or from another file.
    // A symbol named `local N` is private to its file, so it is looked up
    // together with the file.
    let key = |file: usize, symbol: &'_ str| -> (Option<usize>, String) {
        (is_local(symbol).then_some(file), symbol.to_owned())
    };
    let mut definitions: HashMap<(Option<usize>, String), Defined> = HashMap::new();
    for (file, document, source) in &documents {
        let positions = Positions::for_tool(
            source,
            document.position_encoding.enum_value_or_default(),
            tool,
        );
        for occurrence in &document.occurrences {
            if !is_definition(occurrence) || occurrence.symbol.is_empty() {
                continue;
            }
            let Some((start, end)) = byte_range(&positions, occurrence) else {
                continue;
            };
            let defined = match scope_defined_at(index, *file, start, end, source.len()) {
                Some(scope) => Defined::Scope(scope),
                None => {
                    // Not a scope of its own. Inside a function it is a
                    // local variable or a parameter; at module level it is
                    // a binding such as `const { a } = require(...)`.
                    let around = index.kind(index.innermost(*file, start as u32));
                    let module_level = matches!(
                        around,
                        ScopeKind::File
                            | ScopeKind::Document
                            | ScopeKind::Import
                            | ScopeKind::Block
                    );
                    // Only a file-private symbol is certainly a local
                    // variable. A named one, such as an attribute first
                    // assigned in a method, is merely not a scope.
                    if module_level || !is_local(&occurrence.symbol) {
                        Defined::ModuleBinding
                    } else {
                        Defined::Local
                    }
                }
            };
            definitions
                .entry(key(*file, &occurrence.symbol))
                .or_insert(defined);
        }
    }

    let mut imported = Imported::default();
    imported.stats.documents = documents.len() as u32;
    for (file, document, source) in &documents {
        let positions = Positions::for_tool(
            source,
            document.position_encoding.enum_value_or_default(),
            tool,
        );
        let path = index.scopes[*file].path.file.as_str();
        // Parsed only when a kind has to be read off the syntax.
        let mut tree = None;
        // This file's uses that resolved, in order: (start, end, target).
        let mut resolved: Vec<(usize, usize, usize)> = Vec::new();

        for occurrence in &document.occurrences {
            if is_definition(occurrence) || occurrence.symbol.is_empty() {
                continue;
            }
            let Some((start, end)) = byte_range(&positions, occurrence) else {
                continue;
            };
            // `crate`, `self`, `super` and `Self` name where we already are.
            if matches!(&source[start..end], b"crate" | b"self" | b"super" | b"Self") {
                continue;
            }
            imported.stats.uses += 1;
            match definitions.get(&key(*file, &occurrence.symbol)) {
                Some(Defined::Scope(target)) => {
                    let target = resolver.follow_declaration(*target);
                    // A use of an import statement's name says nothing new:
                    // the statement already refers to what it imports.
                    if index.kind(target) == ScopeKind::Import {
                        imported.stats.not_a_scope += 1;
                    } else {
                        resolved.push((start, end, target));
                    }
                }
                Some(Defined::Local) => {
                    imported.locals.insert((*file, start as u32));
                    imported.stats.not_a_scope += 1;
                }
                Some(Defined::ModuleBinding) => imported.stats.not_a_scope += 1,
                None => imported.stats.external += 1,
            }
        }

        resolved.sort();
        for (position, (start, end, target)) in resolved.iter().enumerate() {
            // In `module.name` or `module::name`, the reference is to
            // `name`. The `module` in front is only the way there.
            let is_qualifier =
                resolved
                    .get(position + 1)
                    .is_some_and(|(next_start, _, next_target)| {
                        let between = source.get(*end..*next_start).unwrap_or(&[]);
                        let joiner: Vec<u8> = between
                            .iter()
                            .copied()
                            .filter(|byte| !byte.is_ascii_whitespace())
                            .collect();
                        let is_module = matches!(
                            index.kind(*target),
                            ScopeKind::File
                                | ScopeKind::Document
                                | ScopeKind::Directory
                                | ScopeKind::Module
                        );
                        matches!(joiner.as_slice(), b"." | b"::" | b"?.")
                        && next_target != target
                        // `module.name`: whatever `name` is, the module was
                        // only the way to it. `Type::name` likewise, when
                        // `name` is inside the type.
                        && (is_module || index.is_inside(*next_target, *target))
                    });
            if is_qualifier {
                continue;
            }
            let from = index.innermost(*file, *start as u32);
            let kind = if index.kind(from) == ScopeKind::Import {
                RefKind::Import
            } else if let Some(kind) = fast_kinds.get(&(*file, *start as u32)) {
                *kind
            } else {
                if tree.is_none() {
                    tree = Grammar::for_path(Path::new(path))
                        .and_then(|grammar| mapdiff_extract::parse::parse(grammar, source));
                }
                match &tree {
                    Some(tree) => kind_at(tree.root_node(), *start, *end),
                    None => RefKind::Access,
                }
            };
            if let Some(reference) = resolver.reference(
                from,
                *target,
                kind,
                Tier::Precise,
                *start as u32,
                *end as u32,
            ) {
                let written = std::str::from_utf8(&source[*start..*end]).unwrap_or("");
                let target_name = index.scopes[*target].name.as_deref();
                // `Circle(...)` pointing at `Circle`'s constructor is the
                // class by another route, not another name.
                let constructs = index.kind(*target) == ScopeKind::Constructor
                    && index.parent[*target].and_then(|class| index.scopes[class].name.as_deref())
                        == Some(written);
                // A module named by its path, or by the name an import gave
                // it, is not a rename either.
                let is_module = matches!(
                    index.kind(*target),
                    ScopeKind::File
                        | ScopeKind::Document
                        | ScopeKind::Directory
                        | ScopeKind::Module
                );
                if target_name.is_some_and(|name| name != written) && !constructs && !is_module {
                    imported
                        .renamed
                        .insert((*file, *start as u32), written.to_owned());
                }
                imported.references.push(reference);
                imported.stats.references += 1;
            }
        }
    }
    Ok(imported)
}

/// What a symbol's definition turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Defined {
    /// A scope: uses of the symbol are references to it.
    Scope(usize),
    /// A variable or parameter inside a function. A fast-tier reference
    /// from one of its uses mistook the name for something else.
    Local,
    /// A name that is not a scope but is not a local variable either: a
    /// destructured `require` at module level, an attribute first assigned
    /// in a method. The fast tier's own answer for it stands.
    ModuleBinding,
}

fn byte_range(positions: &Positions<'_>, occurrence: &Occurrence) -> Option<(usize, usize)> {
    let (start_line, start_character, end_line, end_character) = range_of(occurrence)?;
    let start = positions.byte(start_line, start_character)?;
    let end = positions.byte(end_line, end_character)?;
    (start <= end).then_some((start, end))
}

/// The scope a definition names: the one whose name is written there. A
/// definition with no text of its own at the very start of a file, or one
/// that covers the whole file, is the file: indexers define module symbols
/// in one of those two ways.
fn scope_defined_at(
    index: &Index<'_>,
    file: usize,
    start: usize,
    end: usize,
    length: usize,
) -> Option<usize> {
    // rust-analyzer defines a file's module over the whole file instead.
    let whole_file = start == 0 && end + 1 >= length;
    if (start == 0 && end == 0) || whole_file {
        return Some(file);
    }
    let mut current = Some(index.innermost(file, start as u32));
    while let Some(scope) = current {
        if let Some(name) = index.scopes[scope].name_range {
            let (name_start, name_end) = (name.start_byte as usize, name.end_byte as usize);
            // The two need not be identical: `impl Trait for Type` is named
            // by its type, and an indexer may include or leave out a sigil.
            if start < name_end && name_start < end {
                return Some(scope);
            }
        }
        if scope == file {
            break;
        }
        current = index.parent[scope];
    }
    None
}

/// A short description of an index, for working out what an indexer wrote.
/// With `symbol`, lists every occurrence whose symbol contains that text
/// instead.
pub fn describe(bytes: &[u8], symbol: Option<&str>) -> Result<String, String> {
    use std::fmt::Write as _;

    if let Some(wanted) = symbol {
        let parsed =
            ScipIndex::parse_from_bytes(bytes).map_err(|err| format!("not a SCIP index: {err}"))?;
        let mut out = String::new();
        for document in &parsed.documents {
            for occurrence in &document.occurrences {
                if occurrence.symbol.contains(wanted) {
                    let _ = writeln!(
                        out,
                        "{} {:?} {} {}",
                        document.relative_path,
                        range_of(occurrence),
                        if is_definition(occurrence) {
                            "DEFINES"
                        } else {
                            "uses"
                        },
                        occurrence.symbol
                    );
                }
            }
        }
        return Ok(out);
    }

    let parsed =
        ScipIndex::parse_from_bytes(bytes).map_err(|err| format!("not a SCIP index: {err}"))?;
    let mut out = String::new();
    let tool = &parsed.metadata.tool_info;
    let _ = writeln!(out, "tool: {} {}", tool.name, tool.version);
    let _ = writeln!(out, "project root: {}", parsed.metadata.project_root);
    let _ = writeln!(out, "documents: {}", parsed.documents.len());
    for document in &parsed.documents {
        let definitions = document
            .occurrences
            .iter()
            .filter(|occurrence| is_definition(occurrence))
            .count();
        let typed = document
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.typed_range.is_some())
            .count();
        let _ = writeln!(
            out,
            "  {}  {} occurrences ({definitions} definitions, {typed} with typed ranges), encoding {:?}",
            document.relative_path,
            document.occurrences.len(),
            document.position_encoding.enum_value_or_default(),
        );
    }
    if let Some(document) = parsed.documents.first() {
        let _ = writeln!(out, "first occurrences of {}:", document.relative_path);
        for occurrence in document.occurrences.iter().take(12) {
            let _ = writeln!(
                out,
                "  {:?} roles={} {}",
                range_of(occurrence),
                occurrence.symbol_roles,
                occurrence.symbol
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_converted_from_the_index_encoding() {
        // `é` is 2 bytes in UTF-8, 1 unit in UTF-16; `𝒙` is 4 bytes and 2 units.
        let source = "ab\né𝒙z = 1\n".as_bytes();
        let utf8 = Positions::new(source, PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
        let utf16 = Positions::new(source, PositionEncoding::UTF16CodeUnitOffsetFromLineStart);
        let utf32 = Positions::new(source, PositionEncoding::UTF32CodeUnitOffsetFromLineStart);
        // The `z`, which starts at byte 3 + 2 + 4 = 9.
        assert_eq!(utf8.byte(1, 6), Some(9));
        assert_eq!(utf16.byte(1, 3), Some(9));
        assert_eq!(utf32.byte(1, 2), Some(9));
        // Positions outside the file mean the file changed since indexing.
        assert_eq!(utf8.byte(5, 0), None);
        assert_eq!(utf8.byte(0, 99), None);
        // The end of a line is a valid position.
        assert_eq!(utf8.byte(0, 2), Some(2));
    }

    #[test]
    fn both_range_encodings_are_read() {
        let mut packed = Occurrence::new();
        packed.range = vec![3, 4, 9];
        assert_eq!(range_of(&packed), Some((3, 4, 3, 9)));
        packed.range = vec![3, 4, 5, 1];
        assert_eq!(range_of(&packed), Some((3, 4, 5, 1)));
        packed.range = vec![3];
        assert_eq!(range_of(&packed), None);

        let mut typed = Occurrence::new();
        let mut range = scip::types::SingleLineRange::new();
        (range.line, range.start_character, range.end_character) = (7, 2, 6);
        typed.set_single_line_range(range);
        // The typed form wins when both are present.
        typed.range = vec![1, 1, 1];
        assert_eq!(range_of(&typed), Some((7, 2, 7, 6)));
    }
}
