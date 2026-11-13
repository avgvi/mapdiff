//! Builds the reference graph of a repository: which scope uses which.
//!
//! Two tiers find references. The fast tier reads names and imports off the
//! syntax and resolves them without type information; it works on any
//! checkout in well under a second. The precise tier reads what each
//! language's own indexer knows, which adds what needs types, such as a
//! method called on a value.

mod collect_ecma;
mod collect_python;
mod collect_rust;
mod compare;
mod facts;
mod impact;
mod index;
mod map;
mod modules_ecma;
mod modules_python;
mod modules_rust;
mod ops;
mod precise;
mod resolve;
mod scip_import;
mod syntax_kind;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use mapdiff_core::{Position, RefKind, Reference, ReferenceGraph, ScopeKind, ScopeTree};
use mapdiff_extract::Grammar;
use rayon::prelude::*;

pub use compare::{Checkout, Compared, checkout, compare, comparison_map};
pub use impact::{Affected, ImpactOptions, ImpactReport, Impacted, Reason, impact};
pub use map::{Cause, MapContainer, MapData, MapNode, MapWire, Overlay, Presence, union};
pub use ops::{ContainerWires, EntryPoint, ExitPoint, Level, Lifted, Lifter, View};
pub use precise::{Outcome, Precise, TierReport};
pub use scip_import::{ImportStats, describe as describe_scip};

use facts::Facts;
use index::Index;
use resolve::Resolver;

/// A repository's scopes and the references between them.
pub struct Graph {
    pub tree: ScopeTree,
    pub references: ReferenceGraph,
    /// What happened with each indexer of the precise tier. Empty when the
    /// precise tier was off.
    pub tiers: Vec<TierReport>,
    pub comparison: Comparison,
}

/// How the two tiers compare, over the places where both could speak:
/// names the precise tier resolved to a scope in the repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Comparison {
    /// References the precise tier found.
    pub precise: u32,
    /// Of those, how many the fast tier had found too, with the same target.
    pub agreed: u32,
    /// Fast-tier references that the precise tier resolved differently, or
    /// said were local variables. These were wrong and are dropped.
    pub contradicted: u32,
    /// Fast-tier references at places the precise tier said nothing about.
    /// They are kept, marked as fast.
    pub fast_only: u32,
    /// Places where the tiers disagreed and the fast tier was believed: its
    /// target has the name that is written there and the precise tier's
    /// does not. This guards against an indexer defect (see the docs).
    pub overruled: u32,
    /// Precise references that point at a scope with another name than the
    /// one written, at places where the fast tier found nothing to compare
    /// with. Most are aliases; some may be the same indexer defect.
    pub unchecked_renames: u32,
}

/// Extracts the scope tree of everything under `root`, then finds the
/// references between its scopes, using indexes made earlier when they
/// still match the files.
pub fn build(root: &Path) -> io::Result<Graph> {
    build_with(root, Precise::Cached, &|_| {})
}

/// Like `build`, choosing how much of the precise tier to use. `progress`
/// is told before each indexer runs, since one can take minutes.
pub fn build_with(root: &Path, precise: Precise, progress: &dyn Fn(&str)) -> io::Result<Graph> {
    build_from(root, Indexes::Tools(precise, None, progress))
}

/// Like `build_with`, keeping indexes under `cache` instead of in the
/// user's cache directory.
pub fn build_with_cache(
    root: &Path,
    precise: Precise,
    cache: &Path,
    progress: &dyn Fn(&str),
) -> io::Result<Graph> {
    build_from(root, Indexes::Tools(precise, Some(cache), progress))
}

/// A SCIP index made elsewhere, for example by a build pipeline.
pub struct SuppliedIndex {
    /// The directory the indexer ran in, relative to the repository root;
    /// `.` for the root. Paths inside the index are relative to it.
    pub directory: String,
    pub bytes: Vec<u8>,
}

/// Like `build`, with the precise tier read from the given indexes instead
/// of from indexers run here.
pub fn build_with_indexes(root: &Path, indexes: &[SuppliedIndex]) -> io::Result<Graph> {
    build_from(root, Indexes::Supplied(indexes))
}

/// Where the precise tier's indexes come from.
enum Indexes<'a> {
    Tools(Precise, Option<&'a Path>, &'a dyn Fn(&str)),
    Supplied(&'a [SuppliedIndex]),
}

fn build_from(root: &Path, indexes: Indexes<'_>) -> io::Result<Graph> {
    let mapdiff_extract::Extracted { tree, left_out } = mapdiff_extract::extract_repo_parts(root)?;
    let (references, tiers, comparison) = references(root, &tree, &left_out, indexes)?;
    Ok(Graph {
        tree,
        references,
        tiers,
        comparison,
    })
}

fn references(
    root: &Path,
    tree: &ScopeTree,
    left_out: &HashMap<String, Vec<(u32, u32)>>,
    indexes: Indexes<'_>,
) -> io::Result<(ReferenceGraph, Vec<TierReport>, Comparison)> {
    let index = Index::new(tree);

    let files: Vec<usize> = (0..tree.scopes.len())
        .filter(|position| tree.scopes[*position].kind == ScopeKind::File)
        .collect();

    // Read and parse every source file again for its names.
    let collected: Vec<(usize, Facts, Vec<u32>)> = files
        .par_iter()
        .filter_map(|file| {
            let path = tree.scopes[*file].path.file.as_str();
            let grammar = Grammar::for_path(Path::new(path))?;
            let mut source = std::fs::read(root.join(path)).ok()?;
            // Test code inside the file is not on the map, so the names in
            // it must not be read either: it is blanked out, keeping every
            // other byte where it was.
            for (start, end) in left_out.get(path).into_iter().flatten() {
                let end = (*end as usize).min(source.len());
                for byte in &mut source[(*start as usize).min(end)..end] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
            }
            let parsed = mapdiff_extract::parse::parse(grammar, &source)?;
            let node = parsed.root_node();
            let facts = match grammar {
                Grammar::Python => collect_python::collect(node, &source),
                Grammar::Rust => collect_rust::collect(node, &source),
                Grammar::TypeScript | Grammar::Tsx | Grammar::JavaScript => {
                    collect_ecma::collect(node, &source)
                }
            };
            Some((*file, facts, line_starts(&source)))
        })
        .collect();

    let mut lines: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut facts: HashMap<usize, Facts> = HashMap::new();
    for (file, file_facts, file_lines) in collected {
        lines.insert(file, file_lines);
        facts.insert(file, file_facts);
    }

    let resolver = Resolver::new(&index, root, &facts);
    let mut order: Vec<&usize> = facts.keys().collect();
    order.sort();
    let resolved: Vec<_> = order
        .par_iter()
        .map(|file| (**file, resolver.file_references(**file, &facts[*file])))
        .collect();

    let file_count = resolved.len() as u32;
    let unresolved = resolved.iter().map(|(_, (_, unresolved))| unresolved).sum();
    // Fast references, each with the file its referring name is in.
    let mut fast: Vec<(usize, Reference)> = Vec::new();
    for (file, (file_references, _)) in resolved {
        fast.extend(
            file_references
                .into_iter()
                .map(|reference| (file, reference)),
        );
    }

    let mut tiers = Vec::new();
    let mut comparison = Comparison::default();
    let all = if matches!(indexes, Indexes::Tools(Precise::Off, _, _)) {
        fast
    } else {
        let fast_kinds: HashMap<(usize, u32), RefKind> = fast
            .iter()
            .map(|(file, reference)| ((*file, reference.range.start_byte), reference.kind))
            .collect();
        let file_of_id: HashMap<&str, usize> = index
            .scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), index.file_of[position]))
            .collect();

        let mut found: Vec<(usize, Reference)> = Vec::new();
        let mut locals: HashSet<(usize, u32)> = HashSet::new();
        let mut renamed: HashMap<(usize, u32), String> = HashMap::new();
        // Each index to read: which tool made it, where it ran, and its
        // bytes or the reason there are none.
        type Loaded = Result<(Vec<u8>, bool, f32), Outcome>;
        let mut loads: Vec<(&'static str, String, Loaded)> = Vec::new();
        match indexes {
            Indexes::Supplied(supplied) => {
                for index in supplied {
                    let loaded = Ok((index.bytes.clone(), true, 0.0));
                    loads.push(("supplied index", index.directory.clone(), loaded));
                }
            }
            Indexes::Tools(precise, cache, progress) => {
                for project in precise::discover(&index) {
                    let tool = project.tool.program();
                    let loaded = match project.cached(root, &index, cache) {
                        Some(bytes) => Ok((bytes, true, 0.0)),
                        None if precise == Precise::Refresh => {
                            progress(&format!("running {tool} in {}", project.directory));
                            project
                                .run(root, &index, cache)
                                .map(|(bytes, seconds)| (bytes, false, seconds))
                        }
                        None => Err(Outcome::NotIndexed),
                    };
                    loads.push((tool, project.directory, loaded));
                }
            }
        }
        for (tool, directory, loaded) in loads {
            let outcome = match loaded {
                Ok((bytes, from_cache, seconds)) => {
                    match scip_import::import(&bytes, &directory, &resolver, root, &fast_kinds) {
                        Ok(imported) => {
                            for reference in imported.references {
                                let file = file_of_id
                                    .get(reference.from.as_str())
                                    .copied()
                                    .unwrap_or(0);
                                // An indexer reads test code too. A name
                                // used in test code that was left out of
                                // the map is not a use by the file.
                                let at = reference.range.start_byte;
                                let in_test = left_out
                                    .get(tree.scopes[file].path.file.as_str())
                                    .is_some_and(|ranges| {
                                        ranges.iter().any(|(start, end)| *start <= at && at < *end)
                                    });
                                if in_test {
                                    continue;
                                }
                                found.push((file, reference));
                            }
                            locals.extend(imported.locals);
                            renamed.extend(imported.renamed);
                            Outcome::Used {
                                stats: imported.stats,
                                from_cache,
                                seconds,
                            }
                        }
                        Err(message) => Outcome::Failed(message),
                    }
                }
                Err(outcome) => outcome,
            };
            tiers.push(TierReport {
                tool,
                directory,
                outcome,
            });
        }

        let position_of: HashMap<&str, usize> = index
            .scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), position))
            .collect();
        // `new App()` is the class to one tier and its constructor to the
        // other. Both are right, so one inside the other counts as agreeing.
        let related = |a: &str, b: &str| match (position_of.get(a), position_of.get(b)) {
            (Some(a), Some(b)) => index.is_inside(*a, *b) || index.is_inside(*b, *a),
            _ => false,
        };
        let merged;
        let name_of = |id: &str| -> Option<&str> {
            position_of
                .get(id)
                .and_then(|position| index.scopes[*position].name.as_deref())
        };
        (merged, comparison) = merge(fast, found, &locals, &renamed, &related, &name_of);
        merged
    };

    // Byte offsets become lines and columns, once per file.
    let references = all
        .into_iter()
        .map(|(file, mut reference)| {
            if let Some(starts) = lines.get(&file) {
                reference.range.start = position(starts, reference.range.start_byte);
                reference.range.end = position(starts, reference.range.end_byte);
            }
            reference
        })
        .collect();
    Ok((
        ReferenceGraph::new(references, file_count, unresolved),
        tiers,
        comparison,
    ))
}

/// Combines the two tiers. Where the precise tier resolved a name, its
/// answer is the one kept, with one exception below. A fast reference
/// survives where the precise tier said nothing: in a file no indexer
/// covered, inside a macro, and so on. It stays marked as fast, so its
/// lower certainty is visible.
///
/// The exception: when the tiers disagree, and the fast tier's target has
/// exactly the name that is written while the precise tier's has another
/// name, the fast tier is believed. An indexer pointing `Client` at a scope
/// called `AsyncClient` is far more likely to be wrong than a name match is.
fn merge<'a>(
    fast: Vec<(usize, Reference)>,
    precise: Vec<(usize, Reference)>,
    locals: &HashSet<(usize, u32)>,
    renamed: &HashMap<(usize, u32), String>,
    related: &dyn Fn(&str, &str) -> bool,
    name_of: &dyn Fn(&str) -> Option<&'a str>,
) -> (Vec<(usize, Reference)>, Comparison) {
    let mut comparison = Comparison::default();
    let mut kept = Vec::new();
    let mut overruled: HashSet<(usize, u32)> = HashSet::new();
    // Places where the fast tier had an answer of its own.
    let mut compared: HashSet<(usize, u32)> = HashSet::new();
    {
        // What the precise tier says each name refers to.
        let mut answers: HashMap<(usize, u32), HashSet<&str>> = HashMap::new();
        for (file, reference) in &precise {
            answers
                .entry((*file, reference.range.start_byte))
                .or_default()
                .insert(reference.to.as_str());
        }
        for (file, reference) in fast {
            let place = (file, reference.range.start_byte);
            compared.insert(place);
            match answers.get(&place) {
                Some(targets)
                    if targets.iter().any(|target| {
                        *target == reference.to || related(target, &reference.to)
                    }) =>
                {
                    comparison.agreed += 1;
                }
                Some(_) => {
                    let written = renamed.get(&place).map(String::as_str);
                    if written.is_some() && name_of(&reference.to) == written {
                        comparison.overruled += 1;
                        overruled.insert(place);
                        kept.push((file, reference));
                    } else {
                        comparison.contradicted += 1;
                    }
                }
                // An import that binds a module is a local name to some
                // indexers; the import itself is still a true reference.
                None if locals.contains(&place) && reference.kind != RefKind::Import => {
                    comparison.contradicted += 1;
                }
                None => {
                    comparison.fast_only += 1;
                    kept.push((file, reference));
                }
            }
        }
    }
    let before = kept.len();
    comparison.unchecked_renames = precise
        .iter()
        .filter(|(file, reference)| {
            let place = (*file, reference.range.start_byte);
            renamed.contains_key(&place) && !compared.contains(&place)
        })
        .count() as u32;
    kept.extend(
        precise
            .into_iter()
            .filter(|(file, reference)| !overruled.contains(&(*file, reference.range.start_byte))),
    );
    comparison.precise = (kept.len() - before) as u32;
    (kept, comparison)
}

fn line_starts(source: &[u8]) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (offset, byte) in source.iter().enumerate() {
        if *byte == b'\n' {
            starts.push(offset as u32 + 1);
        }
    }
    starts
}

fn position(line_starts: &[u32], byte: u32) -> Position {
    let line = match line_starts.binary_search(&byte) {
        Ok(line) => line,
        Err(next) => next - 1,
    };
    Position {
        line: line as u32,
        column: byte - line_starts[line],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapdiff_core::{Range, Tier};

    fn reference(to: &str, start: u32, kind: RefKind, tier: Tier) -> (usize, Reference) {
        let range = Range {
            start_byte: start,
            end_byte: start + 3,
            ..Range::default()
        };
        (
            0,
            Reference {
                from: String::from("m.rs::f()"),
                to: to.to_owned(),
                kind,
                tier,
                range,
            },
        )
    }

    #[test]
    fn the_precise_tier_overrides_the_fast_one_only_where_it_speaks() {
        let fast = vec![
            reference("m.rs::agreed()", 10, RefKind::Call, Tier::Fast),
            reference("m.rs::wrong()", 20, RefKind::Call, Tier::Fast),
            reference("m.rs::unseen()", 30, RefKind::Call, Tier::Fast),
            reference("m.rs::shadowed()", 40, RefKind::Access, Tier::Fast),
            reference("other.rs", 50, RefKind::Import, Tier::Fast),
            reference("m.rs::written()", 70, RefKind::Call, Tier::Fast),
        ];
        let precise = vec![
            reference("m.rs::agreed()", 10, RefKind::Call, Tier::Precise),
            reference("m.rs::right()", 20, RefKind::Call, Tier::Precise),
            reference("m.rs::method()", 60, RefKind::Call, Tier::Precise),
            reference("m.rs::other_name()", 70, RefKind::Call, Tier::Precise),
        ];
        // The indexer says the names at 40 and 50 are local.
        let locals = HashSet::from([(0, 40), (0, 50)]);

        // At 70 the indexer points a name at a scope called something else,
        // while the fast tier's target has the name that is written.
        let renamed = HashMap::from([((0, 70), String::from("written"))]);
        let name_of = |id: &str| -> Option<&str> { (id == "m.rs::written()").then_some("written") };
        let (merged, comparison) = merge(fast, precise, &locals, &renamed, &|_, _| false, &name_of);
        let mut summary: Vec<(String, Tier)> = merged
            .into_iter()
            .map(|(_, reference)| (reference.to, reference.tier))
            .collect();
        summary.sort();
        assert_eq!(
            summary,
            [
                (String::from("m.rs::agreed()"), Tier::Precise),
                (String::from("m.rs::method()"), Tier::Precise),
                (String::from("m.rs::right()"), Tier::Precise),
                (String::from("m.rs::unseen()"), Tier::Fast),
                (String::from("m.rs::written()"), Tier::Fast),
                (String::from("other.rs"), Tier::Fast),
            ]
        );
        assert_eq!(
            comparison,
            Comparison {
                precise: 3,
                agreed: 1,
                contradicted: 2,
                fast_only: 2,
                overruled: 1,
                unchecked_renames: 0,
            }
        );
    }
}
