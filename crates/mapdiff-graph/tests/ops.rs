//! Lifting references to a level, cycles, bridges and entry points, on a
//! small project where every answer can be worked out by hand.

mod common;

use common::graph;
use mapdiff_core::{Wire, WireKind};
use mapdiff_graph::{Graph, Level, View};

/// `main` uses `core`, `core` uses `util`; `lib/extra` uses `util` from
/// outside the `app` package.
fn project(name: &str) -> Graph {
    graph(
        name,
        &[
            ("app/__init__.py", ""),
            (
                "app/main.py",
                "from app.core import run\n\n\ndef main():\n    run()\n",
            ),
            (
                "app/core.py",
                "from app import util\n\n\ndef run():\n    util.helper()\n    step()\n\n\ndef step():\n    return util.helper()\n",
            ),
            ("app/util.py", "def helper():\n    return 1\n"),
            (
                "lib/extra.py",
                "from app.util import helper\n\n\ndef extra():\n    return helper()\n",
            ),
        ],
    )
}

fn summary(wires: &[Wire]) -> Vec<String> {
    wires
        .iter()
        .map(|wire| {
            let kind = match wire.kind {
                WireKind::Direct => "direct",
                WireKind::Cycle => "cycle",
                WireKind::Bridge => "bridge",
            };
            format!("{} -> {} x{} {kind}", wire.from, wire.to, wire.count)
        })
        .collect()
}

#[test]
fn references_are_lifted_to_files() {
    let graph = project("lift-files");
    let lifted = graph.lift(&View::level(Level::Package));
    assert_eq!(
        summary(&lifted.wires),
        [
            // The import of `util` and two calls of `helper`.
            "app/core.py -> app/util.py x3 direct",
            // The import of `run` and one call.
            "app/main.py -> app/core.py x2 direct",
            "lib/extra.py -> app/util.py x2 direct",
        ]
    );
    // A wire keeps a few of its references as examples.
    assert_eq!(lifted.wires[0].samples.len(), 3);
    assert_eq!(lifted.wires[0].kinds.values().sum::<u32>(), 3);
}

#[test]
fn the_same_references_lift_to_directories_and_to_scopes() {
    let graph = project("lift-levels");

    let architecture = graph.lift(&View::level(Level::Architecture));
    assert_eq!(summary(&architecture.wires), ["lib -> app x2 direct"]);

    let code = graph.lift(&View::level(Level::Code));
    let wires = summary(&code.wires);
    assert!(
        wires.contains(&String::from(
            "app/core.py::run() -> app/util.py::helper() x1 direct"
        )),
        "{wires:?}"
    );
    assert!(
        wires.contains(&String::from(
            "app/core.py::run() -> app/core.py::step() x1 direct"
        )),
        "{wires:?}"
    );
    // Every reference is one wire at this level, or shares one with a twin.
    assert_eq!(
        code.wires.iter().map(|wire| wire.count).sum::<u32>(),
        graph.references.references.len() as u32
    );
}

#[test]
fn each_container_gets_the_wires_among_its_own_children() {
    let graph = project("containers");
    let lifted = graph.lift(&View::level(Level::Package));

    // Inside `app`, between its files.
    assert_eq!(
        summary(&lifted.containers["app"].wires),
        [
            "app/core.py -> app/util.py x3 direct",
            "app/main.py -> app/core.py x2 direct",
        ]
    );
    // At the root, `lib/extra.py -> app/util.py` is a wire between the two
    // directories that hold them.
    assert_eq!(
        summary(&lifted.containers["."].wires),
        ["lib -> app x2 direct"]
    );
    // `lib` has one file and nothing inside it to connect.
    assert!(
        lifted
            .containers
            .get("lib")
            .is_none_or(|lib| lib.wires.is_empty())
    );
}

#[test]
fn entry_points_are_the_children_used_from_outside() {
    let graph = project("entries");
    let lifted = graph.lift(&View::level(Level::Package));
    let entries = &lifted.containers["app"].entries;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].id, "app/util.py");
    assert_eq!((entries[0].incoming, entries[0].from), (2, 1));

    // At scope level the entry point of a file is the function used from
    // other files, with how many scopes use it.
    let code = graph.lift(&View::level(Level::Code));
    let entries = &code.containers["app/util.py"].entries;
    assert_eq!(entries[0].id, "app/util.py::helper()");
    // `run`, `step`, `extra`, and the import statement in `lib/extra.py`.
    assert_eq!(entries[0].from, 4, "{entries:?}");
}

#[test]
fn hiding_a_node_bridges_its_neighbours() {
    let graph = project("bridge");
    let view = View::level(Level::Package).hiding([String::from("app/core.py")]);
    let lifted = graph.lift(&view);
    assert_eq!(
        summary(&lifted.wires),
        [
            // `main` reaches `util` only through the hidden `core`.
            "app/main.py -> app/util.py x2 bridge",
            "lib/extra.py -> app/util.py x2 direct",
        ]
    );
    // Hiding a directory hides what is in it.
    let lifted = graph.lift(&View::level(Level::Package).hiding([String::from("app")]));
    assert!(lifted.wires.is_empty(), "{:?}", summary(&lifted.wires));
}

#[test]
fn a_bridge_is_not_counted_with_references_of_their_own() {
    // `a/one` uses `c` itself; `a/two` reaches it only through `b`.
    let graph = graph(
        "bridge-and-direct",
        &[
            (
                "a/one.py",
                "from c.target import goal\n\n\ndef first():\n    return goal()\n",
            ),
            (
                "a/two.py",
                "from b.mid import middle\n\n\ndef second():\n    return middle()\n",
            ),
            (
                "b/mid.py",
                "from c.target import goal\n\n\ndef middle():\n    return goal()\n",
            ),
            ("c/target.py", "def goal():\n    return 1\n"),
        ],
    );
    let lifted = graph.lift(&View::level(Level::Package).hiding([String::from("b")]));
    assert_eq!(
        summary(&lifted.wires),
        [
            "a/one.py -> c/target.py x2 direct",
            "a/two.py -> c/target.py x2 bridge",
        ]
    );
    // Between the two directories there are two references, not four.
    assert_eq!(summary(&lifted.containers["."].wires), ["a -> c x2 direct"]);
    // And `c` is entered by those two only.
    let entries = &lifted.containers["c"].entries;
    assert_eq!((entries[0].incoming, entries[0].from), (2, 1));

    // With `a/one` hidden as well, the bridge is all there is.
    let view = View::level(Level::Package).hiding([String::from("b"), String::from("a/one.py")]);
    let lifted = graph.lift(&view);
    assert_eq!(summary(&lifted.containers["."].wires), ["a -> c x2 bridge"]);
}

#[test]
fn a_view_can_be_any_set_of_scopes() {
    let graph = project("custom");
    // The package as one node, the other file as another.
    let view = View::scopes([String::from("app"), String::from("lib/extra.py")]);
    assert_eq!(
        summary(&graph.lift(&view).wires),
        ["lib/extra.py -> app x2 direct"]
    );
    // A scope outside every listed one takes no part.
    let view = View::scopes([String::from("app/core.py"), String::from("app/util.py")]);
    assert_eq!(
        summary(&graph.lift(&view).wires),
        ["app/core.py -> app/util.py x3 direct"]
    );
}

#[test]
fn a_loop_among_siblings_has_one_wire_marked_as_closing_it() {
    let graph = graph(
        "cycle",
        &[(
            "m.py",
            "def ping(n):\n    return pong(n) + pong(n - 1)\n\n\ndef pong(n):\n    return ping(n - 1)\n\n\ndef start():\n    return ping(3)\n",
        )],
    );
    let lifted = graph.lift(&View::level(Level::Code));
    assert_eq!(
        summary(&lifted.containers["m.py"].wires),
        [
            // `ping` calls `pong` twice and is called back once: the single
            // call back is the wire that closes the loop.
            "m.py::ping() -> m.py::pong() x2 direct",
            "m.py::pong() -> m.py::ping() x1 cycle",
            "m.py::start() -> m.py::ping() x1 direct",
        ]
    );
    // The same marks on every run.
    assert_eq!(lifted, graph.lift(&View::level(Level::Code)));
}
