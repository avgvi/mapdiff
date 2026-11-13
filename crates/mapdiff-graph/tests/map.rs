//! The map: which scopes are boxes at each level, how they are measured,
//! and the merged map of a comparison.

use std::collections::HashSet;

use mapdiff_gitdiff::{Comparison, Options};
use mapdiff_graph::{Level, MapData, Precise, Presence};
use mapdiff_testkit::TempRepo;

const SHAPES: &str = "\
import math


class Circle:
    radius = 1

    def area(self):
        return math.pi * self.radius**2

    def grow(self, by):
        def clamp(value):
            return max(value, 0)

        self.radius = clamp(self.radius + by)


def unit():
    return Circle()


print(unit().area())
";

const APP: &str = "\
from shapes import Circle, unit


def describe():
    return unit().area()


def enlarge(circle: Circle):
    circle.grow(2)
";

fn ids(map: &MapData) -> Vec<&str> {
    map.nodes.iter().map(|node| node.id.as_str()).collect()
}

fn files() -> Vec<(&'static str, &'static str)> {
    vec![("shapes.py", SHAPES), ("pkg/app.py", APP)]
}

#[test]
fn each_level_shows_its_own_boxes() {
    let dir = std::env::temp_dir().join(format!("mapdiff-map-levels-{}", std::process::id()));
    let graph = graph_in(&dir, &files());
    let none = HashSet::new();

    assert_eq!(
        ids(&graph.map(&dir, Level::Architecture, &none)),
        // A file in the root stands for itself; `pkg/app.py` is part of
        // its directory, and drawn in it as a small mark.
        [".", "pkg", "pkg/app.py", "shapes.py"]
    );
    let architecture = graph.map(&dir, Level::Architecture, &none);
    let minor: Vec<&str> = architecture
        .nodes
        .iter()
        .filter(|node| node.minor)
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(minor, ["pkg/app.py"]);
    // Its references are still its directory's.
    assert_eq!(architecture.containers["."].wires[0].wire.from, "pkg");
    assert_eq!(
        ids(&graph.map(&dir, Level::Package, &none)),
        [".", "pkg", "pkg/app.py", "shapes.py"]
    );
    assert_eq!(
        ids(&graph.map(&dir, Level::Module, &none)),
        [
            ".",
            "pkg",
            "pkg/app.py",
            "pkg/app.py::describe()",
            "pkg/app.py::enlarge()",
            "shapes.py",
            // No import and no loose statement: they count as the file.
            "shapes.py::Circle",
            "shapes.py::Circle::radius",
            "shapes.py::Circle::area()",
            "shapes.py::Circle::grow()",
            "shapes.py::unit()",
        ]
    );
    let code = graph.map(&dir, Level::Code, &none);
    let code_ids = ids(&code);
    assert!(code_ids.contains(&"shapes.py::{import.1}"), "{code_ids:?}");
    assert!(code_ids.contains(&"shapes.py::{block.1}"), "{code_ids:?}");
    // A function is one box, with the function nested in it.
    assert!(code_ids.contains(&"shapes.py::Circle::grow()"));
    assert!(
        !code_ids.iter().any(|id| id.contains("clamp")),
        "{code_ids:?}"
    );

    // Hidden scopes are not on the map, with what is inside them.
    let hidden = HashSet::from([String::from("pkg")]);
    assert_eq!(
        ids(&graph.map(&dir, Level::Package, &hidden)),
        [".", "shapes.py"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn boxes_are_measured_from_the_source() {
    let dir = std::env::temp_dir().join(format!("mapdiff-map-measure-{}", std::process::id()));
    let graph = graph_in(&dir, &files());
    let map = graph.map(&dir, Level::Code, &HashSet::new());
    let node = |id: &str| map.nodes.iter().find(|node| node.id == id).unwrap();

    let grow = node("shapes.py::Circle::grow()");
    assert_eq!(grow.label, "grow");
    assert_eq!((grow.line, grow.lines), (10, 5));
    assert_eq!(
        grow.columns,
        "        self.radius = clamp(self.radius + by)".len() as u32
    );
    assert_eq!(node("shapes.py").lines, 21);
    assert_eq!((node("pkg").lines, node("pkg").columns), (0, 0));
    // A scope with no name is titled by its first line.
    assert_eq!(node("shapes.py::{import.1}").label, "import math");
    assert_eq!(node("shapes.py::{block.1}").label, "print(unit().area())");

    // Wires are between the children of a container, and each container
    // says which children are used from outside and which use the outside.
    let root = &map.containers["."];
    let wires: Vec<(&str, &str)> = root
        .wires
        .iter()
        .map(|wire| (wire.wire.from.as_str(), wire.wire.to.as_str()))
        .collect();
    assert_eq!(wires, [("pkg", "shapes.py")]);
    let entries: Vec<&str> = map.containers["shapes.py"]
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(entries, ["shapes.py::Circle", "shapes.py::unit()"]);
    let exits: Vec<&str> = map.containers["pkg/app.py"]
        .exits
        .iter()
        .map(|exit| exit.id.as_str())
        .collect();
    assert_eq!(
        exits,
        [
            "pkg/app.py::{import.1}",
            "pkg/app.py::describe()",
            "pkg/app.py::enlarge()",
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A graph of files written to `dir`, which the map reads again to measure.
fn graph_in(dir: &std::path::Path, files: &[(&str, &str)]) -> mapdiff_graph::Graph {
    let _ = std::fs::remove_dir_all(dir);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    mapdiff_graph::build_with(dir, Precise::Off, &|_| {}).unwrap()
}

fn comparison_map(repo: &TempRepo, level: Level) -> MapData {
    let git = mapdiff_gitdiff::open(repo.path()).unwrap();
    let comparison = Comparison::parse(&git, None, false).unwrap();
    let changes = mapdiff_gitdiff::change_set(&git, &comparison, Options::default()).unwrap();
    mapdiff_graph::comparison_map(
        &git,
        &comparison,
        &changes,
        level,
        &HashSet::new(),
        Precise::Off,
    )
    .unwrap()
}

fn presence(map: &MapData) -> Vec<(&str, Presence)> {
    map.nodes
        .iter()
        .filter(|node| node.presence != Presence::Both)
        .map(|node| (node.id.as_str(), node.presence))
        .collect()
}

#[test]
fn a_comparison_keeps_what_was_removed_as_ghosts() {
    let repo = TempRepo::new("map-union");
    repo.write("shapes.py", SHAPES);
    repo.write("pkg/app.py", APP);
    repo.commit("base");

    // `describe` goes, `summary` arrives after `enlarge`.
    let start = APP.find("def describe").unwrap();
    let end = APP.find("def enlarge").unwrap();
    repo.write(
        "pkg/app.py",
        &format!(
            "{}{}\n\ndef summary(count):\n    total = 0\n    for index in range(count):\n        total += index\n    return total\n",
            &APP[..start],
            &APP[end..]
        ),
    );

    let map = comparison_map(&repo, Level::Module);
    assert_eq!(
        presence(&map),
        [
            ("pkg/app.py::describe()", Presence::Removed),
            ("pkg/app.py::summary()", Presence::Added),
        ]
    );
    // The ghost is where it was: first in its file, before `enlarge`.
    let in_file: Vec<&str> = map
        .nodes
        .iter()
        .filter(|node| node.parent.as_deref() == Some("pkg/app.py"))
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(
        in_file,
        [
            "pkg/app.py::describe()",
            "pkg/app.py::enlarge()",
            "pkg/app.py::summary()",
        ]
    );
    // A parent always comes before its children.
    let mut seen = HashSet::new();
    for node in &map.nodes {
        if let Some(parent) = &node.parent {
            assert!(
                seen.contains(parent.as_str()),
                "{} before {parent}",
                node.id
            );
        }
        seen.insert(node.id.as_str());
    }
    // The ghost keeps its door: it used something outside its file.
    let exits: Vec<&str> = map.containers["pkg/app.py"]
        .exits
        .iter()
        .map(|exit| exit.id.as_str())
        .collect();
    assert!(exits.contains(&"pkg/app.py::describe()"), "{exits:?}");
}

#[test]
fn wires_are_marked_by_the_side_they_are_on() {
    let repo = TempRepo::new("map-union-wires");
    repo.write(
        "m.py",
        "def a():\n    return b()\n\n\ndef b():\n    return 1\n\n\ndef c():\n    return 2\n",
    );
    repo.commit("base");
    // `a` stops using `b` and uses `c`; `b` now uses `a`.
    repo.write(
        "m.py",
        "def a():\n    return c()\n\n\ndef b():\n    return a()\n\n\ndef c():\n    return 2\n",
    );

    let map = comparison_map(&repo, Level::Module);
    let wires: Vec<(String, Presence, mapdiff_core::WireKind)> = map.containers["m.py"]
        .wires
        .iter()
        .map(|wire| {
            (
                format!("{} -> {}", wire.wire.from, wire.wire.to),
                wire.presence,
                wire.wire.kind,
            )
        })
        .collect();
    use mapdiff_core::WireKind::{Cycle, Direct};
    assert_eq!(
        wires,
        [
            // With both sides drawn, `a` and `b` use each other: one of
            // the two wires is marked as closing the loop.
            (
                String::from("m.py::a() -> m.py::b()"),
                Presence::Removed,
                Direct
            ),
            (
                String::from("m.py::a() -> m.py::c()"),
                Presence::Added,
                Direct
            ),
            (
                String::from("m.py::b() -> m.py::a()"),
                Presence::Added,
                Cycle
            ),
        ]
    );
    assert!(presence(&map).is_empty());
}

#[test]
fn a_renamed_scope_is_one_box_and_its_children_follow() {
    let repo = TempRepo::new("map-union-rename");
    repo.write("shapes.py", SHAPES);
    repo.commit("base");
    repo.replace("shapes.py", "class Circle:", "class Disc:");
    repo.replace("shapes.py", "return Circle()", "return Disc()");

    let map = comparison_map(&repo, Level::Module);
    let listed = ids(&map);
    assert!(listed.contains(&"shapes.py::Disc"), "{listed:?}");
    assert!(listed.contains(&"shapes.py::Disc::area()"), "{listed:?}");
    assert!(!listed.iter().any(|id| id.contains("Circle")), "{listed:?}");
    // Nothing was added or removed: the same boxes under a new name.
    assert!(presence(&map).is_empty(), "{:?}", presence(&map));
}

#[test]
fn with_no_change_the_merged_map_is_the_map() {
    let repo = TempRepo::new("map-union-same");
    repo.write("shapes.py", SHAPES);
    repo.write("pkg/app.py", APP);
    repo.commit("base");

    let merged = comparison_map(&repo, Level::Code);
    let plain = mapdiff_graph::build_with(repo.path(), Precise::Off, &|_| {})
        .unwrap()
        .map(repo.path(), Level::Code, &HashSet::new());
    // The same boxes and wires; the overlay is there and empty.
    assert_eq!(merged.nodes, plain.nodes);
    assert_eq!(
        merged.nodes[0].label,
        repo.path()
            .canonicalize()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
    );
    assert_eq!(merged.containers, plain.containers);
    assert_eq!(merged.overlay, Some(mapdiff_graph::Overlay::default()));
}

/// The comparison of the uncommitted changes, with its change set.
fn compared(repo: &TempRepo, level: Level) -> (MapData, mapdiff_core::ChangeSet) {
    let git = mapdiff_gitdiff::open(repo.path()).unwrap();
    let comparison = Comparison::parse(&git, None, false).unwrap();
    let changes = mapdiff_gitdiff::change_set(&git, &comparison, Options::default()).unwrap();
    let map = mapdiff_graph::comparison_map(
        &git,
        &comparison,
        &changes,
        level,
        &HashSet::new(),
        Precise::Off,
    )
    .unwrap();
    (map, changes)
}

const GEOMETRY: &str = "\
def area(width, height):
    return width * height


def perimeter(width, height):
    return 2 * (width + height)


def ratio(width, height):
    return area(width, height) / perimeter(width, height)
";

const REPORT: &str = "\
from geometry import area, ratio


def describe(width, height):
    return f\"{area(width, height)}\"


def summary(width, height):
    return f\"{describe(width, height)} {ratio(width, height)}\"


def unrelated():
    return 1
";

#[test]
fn each_change_is_drawn_in_the_box_its_scope_is_in() {
    let repo = TempRepo::new("overlay-boxes");
    repo.write("geometry.py", GEOMETRY);
    repo.write("report.py", REPORT);
    repo.commit("base");
    repo.replace(
        "geometry.py",
        "def area(width, height):",
        "def area(width, height, scale=1):",
    );
    repo.replace("report.py", "    return 1\n", "    return 2\n");

    let labels = |map: &MapData, changes: &mapdiff_core::ChangeSet, node: &str| -> Vec<String> {
        map.overlay.as_ref().unwrap().changes[node]
            .iter()
            .map(|position| changes.changes[*position as usize].label.clone())
            .collect()
    };
    // Where functions are boxes, each change is in its own.
    let (map, changes) = compared(&repo, Level::Module);
    let overlay = map.overlay.as_ref().unwrap();
    assert_eq!(
        overlay.changes.keys().collect::<Vec<_>>(),
        ["geometry.py::area()", "report.py::unrelated()"]
    );
    assert_eq!(labels(&map, &changes, "geometry.py::area()"), ["area()"]);
    // Where files are the boxes, a file holds the changes inside it.
    let (map, changes) = compared(&repo, Level::Package);
    assert_eq!(labels(&map, &changes, "geometry.py"), ["area()"]);
    assert_eq!(labels(&map, &changes, "report.py"), ["unrelated()"]);
}

#[test]
fn neighbours_and_affected_users_are_told_apart() {
    let repo = TempRepo::new("overlay-impact");
    repo.write("geometry.py", GEOMETRY);
    repo.write("report.py", REPORT);
    repo.commit("base");
    // `area` gets another parameter; `perimeter` only a new body.
    repo.replace(
        "geometry.py",
        "def area(width, height):",
        "def area(width, height, scale=1):",
    );
    repo.replace(
        "geometry.py",
        "    return 2 * (width + height)\n",
        "    return (width + height) * 2\n",
    );

    let (map, _) = compared(&repo, Level::Module);
    let overlay = map.overlay.as_ref().unwrap();
    // `ratio` uses both; `describe` uses `area`. Neither changed.
    assert_eq!(
        overlay.near,
        ["geometry.py::ratio()", "report.py::describe()"]
    );
    // Both can see the new signature of `area`; nobody has to care about
    // the body of `perimeter`.
    let affected: Vec<(&str, &str, &str)> = overlay
        .impacted
        .iter()
        .flat_map(|(node, causes)| {
            causes
                .iter()
                .map(move |cause| (node.as_str(), cause.at.as_str(), cause.reasons[0].as_str()))
        })
        .collect();
    assert_eq!(
        affected,
        [
            (
                "geometry.py::ratio()",
                "geometry.py::area()",
                "signature changed"
            ),
            (
                "report.py::describe()",
                "geometry.py::area()",
                "signature changed"
            ),
        ]
    );
    // `summary` and `unrelated` are neither.
    assert!(!overlay.near.iter().any(|node| node.contains("summary")));

    // At the level of files, a file with a change in it is not a neighbour
    // of itself.
    let (map, _) = compared(&repo, Level::Package);
    let overlay = map.overlay.as_ref().unwrap();
    assert_eq!(overlay.near, ["report.py"]);
    assert_eq!(overlay.impacted["report.py"][0].at, "geometry.py");
}

#[test]
fn changes_are_read_from_what_is_used_to_what_uses_it() {
    let repo = TempRepo::new("overlay-order");
    repo.write("geometry.py", GEOMETRY);
    repo.write("report.py", REPORT);
    repo.write("CHANGES.md", "# Changes\n");
    repo.commit("base");
    repo.write("CHANGES.md", "# Changes\n\n- Units.\n");
    // Four changes: `summary` uses `describe` and `ratio`; `ratio` uses `area`.
    repo.replace(
        "geometry.py",
        "    return width * height\n",
        "    return height * width\n",
    );
    repo.replace(
        "geometry.py",
        "    return area(width, height) / perimeter(width, height)\n",
        "    return area(width, height) / (perimeter(width, height) or 1)\n",
    );
    repo.replace(
        "report.py",
        "def describe(width, height):",
        "def describe(width, height, unit=\"\"):",
    );
    repo.replace(
        "report.py",
        "def summary(width, height):",
        "def summary(width, height, sep=\" \"):",
    );

    let (map, changes) = compared(&repo, Level::Module);
    let order: Vec<&str> = map
        .overlay
        .as_ref()
        .unwrap()
        .order
        .iter()
        .map(|position| changes.changes[*position as usize].label.as_str())
        .collect();
    // The changelog sorts first by name and is read last: prose after code.
    assert_eq!(
        order,
        ["area()", "ratio()", "describe()", "summary()", "CHANGES.md"]
    );
    // The same on every run, at every level.
    let (again, _) = compared(&repo, Level::Package);
    assert_eq!(again.overlay.unwrap().order, map.overlay.unwrap().order);
}

#[test]
fn a_moved_scope_is_a_ghost_where_it_was_and_a_box_where_it_is() {
    let repo = TempRepo::new("overlay-move");
    repo.write("geometry.py", GEOMETRY);
    repo.write("report.py", REPORT);
    repo.commit("base");
    // `perimeter` goes to another file, unchanged.
    let start = GEOMETRY.find("def perimeter").unwrap();
    let end = GEOMETRY.find("def ratio").unwrap();
    repo.write(
        "geometry.py",
        &format!("{}{}", &GEOMETRY[..start], &GEOMETRY[end..]),
    );
    repo.write("edges.py", GEOMETRY[start..end].trim_end());

    let (map, changes) = compared(&repo, Level::Module);
    assert_eq!(
        presence(&map),
        [
            ("edges.py", Presence::Added),
            ("edges.py::perimeter()", Presence::Added),
            ("geometry.py::perimeter()", Presence::Removed),
        ]
    );
    let overlay = map.overlay.as_ref().unwrap();
    let state = |node: &str| changes.changes[overlay.changes[node][0] as usize].state;
    assert_eq!(
        state("geometry.py::perimeter()"),
        mapdiff_core::ChangeState::MovedOut
    );
    assert_eq!(
        state("edges.py::perimeter()"),
        mapdiff_core::ChangeState::MovedIn
    );
    // The two ends name each other, so a viewer can join them.
    let out = &changes.changes[overlay.changes["geometry.py::perimeter()"][0] as usize];
    assert_eq!(out.counterpart.as_deref(), Some("edges.py::perimeter()"));
}

const LIBRARY: &str = "\
pub fn double(value: i32) -> i32 {
    value * 2
}

pub fn quadruple(value: i32) -> i32 {
    double(double(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles() {
        assert_eq!(double(2), 4);
    }
}
";

#[test]
fn tests_are_not_on_the_map_and_their_uses_are_not_counted() {
    let dir = std::env::temp_dir().join(format!("mapdiff-map-no-tests-{}", std::process::id()));
    let graph = graph_in(
        &dir,
        &[
            ("src/lib.rs", LIBRARY),
            (
                "tests/api.rs",
                "use demo::double;\n\n#[test]\nfn from_outside() {\n    assert_eq!(double(1), 2);\n}\n",
            ),
        ],
    );
    let map = graph.map(&dir, Level::Code, &HashSet::new());
    // Neither the test file nor the test module inside the source file.
    assert_eq!(
        ids(&map),
        [
            ".",
            "src",
            "src/lib.rs",
            "src/lib.rs::double()",
            "src/lib.rs::quadruple()"
        ]
    );
    // `double` is used twice, both times by `quadruple`. The uses in the
    // test module are not uses by the file it sits in.
    let users: Vec<(&str, &str)> = graph
        .references
        .references
        .iter()
        .map(|reference| (reference.from.as_str(), reference.to.as_str()))
        .collect();
    assert_eq!(
        users,
        [
            ("src/lib.rs::quadruple()", "src/lib.rs::double()"),
            ("src/lib.rs::quadruple()", "src/lib.rs::double()"),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_test_is_listed_but_has_no_box() {
    let repo = TempRepo::new("overlay-tests");
    repo.write("src/lib.rs", LIBRARY);
    repo.write("README.md", "# Demo\n");
    repo.write(
        "tests/api.rs",
        "#[test]\nfn from_outside() {\n    assert!(true);\n}\n",
    );
    repo.commit("base");
    // A function, the test inside its file, a test file and the readme.
    repo.replace("src/lib.rs", "    value * 2\n", "    value + value\n");
    repo.replace(
        "src/lib.rs",
        "assert_eq!(double(2), 4);",
        "assert_eq!(double(3), 6);",
    );
    repo.replace("tests/api.rs", "assert!(true);", "assert!(1 == 1);");
    repo.write("README.md", "# Demo\n\nDoubles numbers.\n");

    let (map, changes) = compared(&repo, Level::Module);
    let overlay = map.overlay.as_ref().unwrap();
    let label = |position: &u32| changes.changes[*position as usize].label.as_str();
    // Only the function has a box.
    assert_eq!(
        overlay.changes.keys().collect::<Vec<_>>(),
        ["README.md", "src/lib.rs::double()"]
    );
    assert_eq!(
        overlay.tests.iter().map(label).collect::<Vec<_>>(),
        ["doubles()", "from_outside()"]
    );
    // Read in this order: code, then tests, then prose.
    assert_eq!(
        overlay.order.iter().map(label).collect::<Vec<_>>(),
        ["double()", "doubles()", "from_outside()", "README.md"]
    );
}
