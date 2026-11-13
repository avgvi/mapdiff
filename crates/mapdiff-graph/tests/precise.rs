//! The precise tier: reading an index, merging it with the fast tier, and
//! running the real indexers.

use std::collections::BTreeSet;
use std::process::Command;

use mapdiff_core::{RefKind, Tier};
use mapdiff_graph::{Graph, Outcome, Precise, SuppliedIndex};
use mapdiff_testkit::TempRepo;
use protobuf::{Enum, Message};
use scip::types::{Document, Index, Metadata, Occurrence, PositionEncoding, SymbolRole, ToolInfo};

/// One occurrence of `text` on a 0-based line, found by searching the line.
/// `nth` picks among several on the same line.
fn occurrence(
    source: &str,
    line: usize,
    text: &str,
    nth: usize,
    symbol: &str,
    definition: bool,
) -> Occurrence {
    let line_text = source.lines().nth(line).expect("line exists");
    let column = line_text
        .match_indices(text)
        .nth(nth)
        .unwrap_or_else(|| panic!("{text:?} occurs {} times on line {line}", nth + 1))
        .0;
    // Columns are UTF-16 units here, as scip-typescript writes them.
    let units = |upto: usize| line_text[..upto].encode_utf16().count() as i32;
    let mut occurrence = Occurrence::new();
    occurrence.range = vec![line as i32, units(column), units(column + text.len())];
    occurrence.symbol = symbol.to_owned();
    if definition {
        occurrence.symbol_roles = SymbolRole::Definition.value();
    }
    occurrence
}

fn index_of(path: &str, occurrences: Vec<Occurrence>) -> Vec<u8> {
    let mut document = Document::new();
    document.relative_path = path.to_owned();
    document.position_encoding = PositionEncoding::UTF16CodeUnitOffsetFromLineStart.into();
    document.occurrences = occurrences;

    let mut tool = ToolInfo::new();
    tool.name = String::from("hand-written");
    let mut metadata = Metadata::new();
    metadata.tool_info = Some(tool).into();
    let mut index = Index::new();
    index.metadata = Some(metadata).into();
    index.documents = vec![document];
    index.write_to_bytes().expect("index serialises")
}

fn edges(graph: &Graph) -> BTreeSet<String> {
    graph
        .references
        .references
        .iter()
        .map(|reference| {
            let tier = match reference.tier {
                Tier::Fast => "fast",
                Tier::Precise => "precise",
            };
            format!("{} -> {} [{tier}]", reference.from, reference.to)
        })
        .collect()
}

const SHAPES: &str = "\
class Shape:
    def area(self):
        return 0


def helper(value):
    return value


def total(shape, helper):
    # é: a character that is one UTF-16 unit and two bytes
    size = shape.area()
    return helper(size)
";

#[test]
fn an_index_adds_what_needs_types_and_corrects_the_fast_tier() {
    let repo = TempRepo::new("supplied");
    repo.write("shapes.py", SHAPES);

    // The fast tier alone cannot resolve `shape.area()`: it does not know
    // what `shape` is.
    let fast = mapdiff_graph::build_with(repo.path(), Precise::Off, &|_| {}).unwrap();
    assert!(
        !edges(&fast)
            .iter()
            .any(|edge| edge.contains("Shape::area()")),
        "{:?}",
        edges(&fast)
    );

    let area = "pkg `shapes`/Shape#area().";
    let index = index_of(
        "shapes.py",
        vec![
            occurrence(SHAPES, 0, "Shape", 0, "pkg `shapes`/Shape#", true),
            occurrence(SHAPES, 1, "area", 0, area, true),
            occurrence(SHAPES, 5, "helper", 0, "pkg `shapes`/helper().", true),
            occurrence(SHAPES, 9, "total", 0, "pkg `shapes`/total().", true),
            // Parameters and local variables are defined, but are not scopes.
            occurrence(SHAPES, 9, "shape", 0, "pkg `shapes`/total().(shape)", true),
            occurrence(SHAPES, 9, "helper", 0, "local 1", true),
            occurrence(SHAPES, 11, "size", 0, "local 2", true),
            // Uses.
            occurrence(
                SHAPES,
                11,
                "shape",
                0,
                "pkg `shapes`/total().(shape)",
                false,
            ),
            occurrence(SHAPES, 11, "area", 0, area, false),
            occurrence(SHAPES, 12, "helper", 0, "local 1", false),
            occurrence(SHAPES, 12, "size", 0, "local 2", false),
            // Something from outside the repository.
            occurrence(SHAPES, 6, "value", 0, "pkg `builtins`/int#", false),
        ],
    );
    let supplied = [SuppliedIndex {
        directory: String::from("."),
        bytes: index,
    }];
    let graph = mapdiff_graph::build_with_indexes(repo.path(), &supplied).unwrap();

    assert_eq!(
        edges(&graph),
        BTreeSet::from([String::from(
            "shapes.py::total() -> shapes.py::Shape::area() [precise]"
        )])
    );
    let reference = &graph.references.references[0];
    assert_eq!(reference.kind, RefKind::Call);
    // The position is in bytes and lines of the file, not index columns:
    // `area` on line 12 (1-based), after the two-byte `é` on the line above.
    assert_eq!(reference.range.start.line, 11);
    assert_eq!(
        &SHAPES[reference.range.start_byte as usize..reference.range.end_byte as usize],
        "area"
    );

    match &graph.tiers[0].outcome {
        Outcome::Used { stats, .. } => {
            assert_eq!(stats.documents, 1);
            assert_eq!(stats.uses, 5);
            assert_eq!(stats.references, 1);
            assert_eq!(stats.external, 1);
            assert_eq!(stats.not_a_scope, 3);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_index_for_a_file_that_changed_is_not_trusted() {
    let repo = TempRepo::new("stale");
    repo.write("shapes.py", SHAPES);
    // Line 40 does not exist: the index describes another version of the file.
    let mut stale = Occurrence::new();
    stale.range = vec![40, 0, 4];
    stale.symbol = String::from("pkg `shapes`/total().");
    let supplied = [SuppliedIndex {
        directory: String::from("."),
        bytes: index_of("shapes.py", vec![stale]),
    }];
    let graph = mapdiff_graph::build_with_indexes(repo.path(), &supplied).unwrap();
    assert!(
        graph
            .references
            .references
            .iter()
            .all(|reference| reference.tier == Tier::Fast)
    );
}

#[test]
fn bytes_that_are_not_an_index_are_reported_not_fatal() {
    let repo = TempRepo::new("garbage");
    repo.write("shapes.py", SHAPES);
    let supplied = [SuppliedIndex {
        directory: String::from("."),
        bytes: vec![0xff, 0xff, 0xff, 0xff, 0xff],
    }];
    let graph = mapdiff_graph::build_with_indexes(repo.path(), &supplied).unwrap();
    assert!(
        matches!(graph.tiers[0].outcome, Outcome::Failed(_)),
        "{:?}",
        graph.tiers
    );
}

fn installed(program: &str) -> bool {
    // rustup puts a stand-in on the path for a component that is not
    // installed, and it exits with an error.
    let found = Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipping: {program} is not installed");
    }
    found
}

/// Runs the real indexers on a throwaway project and returns the graph.
/// Indexes are kept inside the throwaway directory's own cache folder.
fn indexed(name: &str, files: &[(&str, &str)]) -> (TempRepo, Graph) {
    let repo = TempRepo::new(name);
    for (path, content) in files {
        repo.write(path, content);
    }
    let cache = repo.path().join(".git/mapdiff-test-cache");
    let graph =
        mapdiff_graph::build_with_cache(repo.path(), Precise::Refresh, &cache, &|_| {}).unwrap();
    for tier in &graph.tiers {
        assert!(
            matches!(
                tier.outcome,
                Outcome::Used {
                    from_cache: false,
                    ..
                }
            ),
            "{} did not run cleanly: {:?}",
            tier.tool,
            tier.outcome
        );
    }

    // A second build reuses the index instead of running the indexer again,
    // and gives the same graph.
    let again =
        mapdiff_graph::build_with_cache(repo.path(), Precise::Cached, &cache, &|_| {}).unwrap();
    for tier in &again.tiers {
        assert!(
            matches!(
                tier.outcome,
                Outcome::Used {
                    from_cache: true,
                    ..
                }
            ),
            "{:?}",
            tier.outcome
        );
    }
    assert_eq!(graph.references, again.references);

    // The indexer left nothing behind in the project.
    let status = repo.git(&["status", "--porcelain", "--ignored"]);
    let stray: Vec<&str> = status
        .lines()
        .filter(|line| {
            let path = line[3..].trim_end_matches('/');
            !files.iter().any(|(file, _)| *file == path || file.starts_with(&format!("{path}/")))
                // Cargo's build directory and lock file are its own business.
                && !matches!(path, "target" | "Cargo.lock")
        })
        .collect();
    assert!(stray.is_empty(), "left behind: {stray:?}");
    (repo, graph)
}

fn has(graph: &Graph, edge: &str) -> bool {
    edges(graph).contains(edge)
}

#[test]
fn python_indexer_finds_a_method_called_on_a_value() {
    if !installed("scip-python") {
        return;
    }
    let (repo, graph) = indexed(
        "scip-python",
        &[(
            "shapes.py",
            "class Shape:\n    def area(self) -> float:\n        return 0.0\n\n\ndef total(shape: Shape) -> float:\n    return shape.area()\n",
        )],
    );
    assert!(
        has(
            &graph,
            "shapes.py::total() -> shapes.py::Shape::area() [precise]"
        ),
        "{:?}",
        edges(&graph)
    );

    // Editing a file makes its index stale: the next build says so and
    // falls back to the fast tier instead of using positions that moved.
    repo.replace("shapes.py", "return 0.0", "return 1.0");
    let cache = repo.path().join(".git/mapdiff-test-cache");
    let stale =
        mapdiff_graph::build_with_cache(repo.path(), Precise::Cached, &cache, &|_| {}).unwrap();
    assert_eq!(stale.tiers[0].outcome, Outcome::NotIndexed);
    assert!(
        stale
            .references
            .references
            .iter()
            .all(|reference| reference.tier == Tier::Fast)
    );
}

#[test]
fn typescript_indexer_finds_methods_through_types_and_plain_javascript() {
    if !installed("scip-typescript") {
        return;
    }
    let (_repo, graph) = indexed(
        "scip-typescript",
        &[
            (
                "tsconfig.json",
                "{ \"compilerOptions\": { \"strict\": true, \"noEmit\": true }, \"include\": [\"src\"] }\n",
            ),
            (
                "src/shapes.ts",
                "export interface Shape {\n  area(): number;\n}\n\nexport function total(shape: Shape): number {\n  return shape.area();\n}\n",
            ),
        ],
    );
    assert!(
        has(
            &graph,
            "src/shapes.ts::total() -> src/shapes.ts::Shape::area() [precise]"
        ),
        "{:?}",
        edges(&graph)
    );

    // No tsconfig: the indexer is asked to make one up, and must not leave
    // it in the project (checked inside `indexed`).
    let (_repo, graph) = indexed(
        "scip-javascript",
        &[(
            "lib/shapes.js",
            "class Shape {\n  area() {\n    return 0;\n  }\n}\n\nfunction total() {\n  const shape = new Shape();\n  return shape.area();\n}\n\nmodule.exports = { total };\n",
        )],
    );
    assert!(
        has(
            &graph,
            "lib/shapes.js::total() -> lib/shapes.js::Shape::area() [precise]"
        ),
        "{:?}",
        edges(&graph)
    );
}

#[test]
fn rust_indexer_finds_a_trait_method_called_on_a_value() {
    if !installed("rust-analyzer") {
        return;
    }
    let (_repo, graph) = indexed(
        "rust-analyzer",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            (
                "src/lib.rs",
                "pub trait Shape {\n    fn area(&self) -> f64;\n}\n\npub struct Circle {\n    pub radius: f64,\n}\n\nimpl Shape for Circle {\n    fn area(&self) -> f64 {\n        self.radius\n    }\n}\n\npub fn total(shape: &dyn Shape, circle: &Circle) -> f64 {\n    shape.area() + circle.radius\n}\n",
            ),
        ],
    );
    // A call through `dyn Shape` goes to the trait's method.
    assert!(
        has(
            &graph,
            "src/lib.rs::total() -> src/lib.rs::Shape::area() [precise]"
        ),
        "{:?}",
        edges(&graph)
    );
    // A field read on a value.
    assert!(
        has(
            &graph,
            "src/lib.rs::total() -> src/lib.rs::Circle::radius [precise]"
        ),
        "{:?}",
        edges(&graph)
    );
}
