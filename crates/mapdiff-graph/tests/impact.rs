//! The impact of a change: unchanged scopes that use what changed.

use mapdiff_gitdiff::{Comparison, Options};
use mapdiff_graph::{ImpactOptions, ImpactReport, Precise, Reason};
use mapdiff_testkit::TempRepo;

const SHAPES: &str = "\
def area(width, height):
    return width * height


def perimeter(width, height):
    return 2 * (width + height)


def unused():
    return 0
";

const APP: &str = "\
from shapes import area, perimeter


def describe(width, height):
    return f\"{area(width, height)} {perimeter(width, height)}\"


def report():
    return describe(2, 3)
";

fn repo(name: &str) -> TempRepo {
    let repo = TempRepo::new(name);
    repo.write("shapes.py", SHAPES);
    repo.write("app.py", APP);
    repo.commit("base");
    repo
}

/// The impact of the uncommitted changes, with the graph before the change
/// built from an export of the last commit.
fn impact_of(repo: &TempRepo, options: ImpactOptions) -> ImpactReport {
    let git = mapdiff_gitdiff::open(repo.path()).unwrap();
    let comparison = Comparison::parse(&git, None, false).unwrap();
    let changes = mapdiff_gitdiff::change_set(&git, &comparison, Options::default()).unwrap();
    let head = mapdiff_graph::build_with(repo.path(), Precise::Off, &|_| {}).unwrap();

    let export = repo.path().join(".git/mapdiff-test-export");
    let _ = std::fs::remove_dir_all(&export);
    git.export(&comparison.base_side(), &export).unwrap();
    let base = mapdiff_graph::build_with(&export, Precise::Off, &|_| {}).unwrap();
    mapdiff_graph::impact(&changes, &head, Some(&base), options)
}

#[test]
fn a_signature_change_lists_the_unchanged_scopes_that_call_it() {
    let repo = repo("signature");
    repo.replace(
        "shapes.py",
        "def area(width, height):",
        "def area(width, height, scale=1):",
    );

    let report = impact_of(&repo, ImpactOptions::default());
    assert_eq!(report.visible_changes, 1);
    assert_eq!(report.impacted.len(), 1);
    let impacted = &report.impacted[0];
    assert_eq!(impacted.id, "shapes.py::area()");
    assert_eq!(impacted.reasons, [Reason::Signature]);

    let users: Vec<(&str, u32, bool)> = impacted
        .affected
        .iter()
        .map(|user| (user.id.as_str(), user.distance, user.also_changed))
        .collect();
    assert_eq!(
        users,
        [
            // The call itself, and the import statement that names it.
            ("app.py::describe()", 1, false),
            ("app.py::{import.1}", 1, false),
        ]
    );
    // A direct user says where it uses the changed scope.
    assert_eq!(impacted.affected[0].references.len(), 1);
    assert_eq!(impacted.affected[0].references[0].range.start.line, 4);
    assert_eq!(report.unchanged_users, 2);
}

#[test]
fn depth_follows_users_of_users() {
    let repo = repo("depth");
    repo.replace(
        "shapes.py",
        "def area(width, height):",
        "def area(width, height, scale=1):",
    );

    let report = impact_of(
        &repo,
        ImpactOptions {
            depth: 2,
            bodies: false,
        },
    );
    let users: Vec<(&str, u32)> = report.impacted[0]
        .affected
        .iter()
        .map(|user| (user.id.as_str(), user.distance))
        .collect();
    assert_eq!(
        users,
        [
            ("app.py::describe()", 1),
            ("app.py::{import.1}", 1),
            ("app.py::report()", 2),
        ]
    );
}

#[test]
fn a_body_change_affects_nobody_unless_asked() {
    let repo = repo("body");
    repo.replace(
        "shapes.py",
        "    return width * height\n",
        "    return height * width\n",
    );

    let quiet = impact_of(&repo, ImpactOptions::default());
    assert!(quiet.impacted.is_empty());
    assert_eq!(quiet.visible_changes, 0);

    let asked = impact_of(
        &repo,
        ImpactOptions {
            depth: 1,
            bodies: true,
        },
    );
    assert_eq!(asked.impacted[0].reasons, [Reason::Body]);
    assert_eq!(asked.impacted[0].affected.len(), 2);
}

#[test]
fn a_removed_scope_lists_who_used_it_before() {
    let repo = repo("removed");
    let start = SHAPES.find("def perimeter").unwrap();
    let end = SHAPES.find("def unused").unwrap();
    repo.write(
        "shapes.py",
        &format!("{}{}", &SHAPES[..start], &SHAPES[end..]),
    );

    let report = impact_of(&repo, ImpactOptions::default());
    let impacted = &report.impacted[0];
    assert_eq!(impacted.id, "shapes.py::perimeter()");
    assert_eq!(impacted.reasons, [Reason::Removed]);
    // `describe` still calls it and was not touched: it is now broken.
    let users: Vec<(&str, bool)> = impacted
        .affected
        .iter()
        .map(|user| (user.id.as_str(), user.also_changed))
        .collect();
    assert_eq!(
        users,
        [("app.py::describe()", false), ("app.py::{import.1}", false)]
    );
}

#[test]
fn users_that_are_part_of_the_change_are_told_apart() {
    let repo = repo("also-changed");
    repo.replace(
        "shapes.py",
        "def area(width, height):",
        "def area(width, height, scale=1):",
    );
    // The caller is updated in the same change.
    repo.replace("app.py", "area(width, height)", "area(width, height, 2)");

    let report = impact_of(&repo, ImpactOptions::default());
    let describe = report.impacted[0]
        .affected
        .iter()
        .find(|user| user.id == "app.py::describe()")
        .unwrap();
    assert!(describe.also_changed);
    // Only the import statement is left as an unchanged user.
    assert_eq!(report.unchanged_users, 1);
}

#[test]
fn changes_nobody_uses_and_added_scopes_have_no_impact() {
    let repo = repo("no-users");
    repo.replace("shapes.py", "def unused():", "def unused(flag=False):");
    repo.replace(
        "shapes.py",
        "def unused(flag=False):\n    return 0\n",
        "def unused(flag=False):\n    return 0\n\n\ndef fresh():\n    return 1\n",
    );

    let report = impact_of(&repo, ImpactOptions::default());
    // One visible change (`unused`); the added scope is not one.
    assert_eq!(report.visible_changes, 1);
    assert!(report.impacted.is_empty());
    assert_eq!(report.unchanged_users, 0);
}

#[test]
fn without_the_graph_from_before_removals_are_flagged_not_guessed() {
    let repo = repo("no-base");
    let start = SHAPES.find("def perimeter").unwrap();
    let end = SHAPES.find("def unused").unwrap();
    repo.write(
        "shapes.py",
        &format!("{}{}", &SHAPES[..start], &SHAPES[end..]),
    );

    let git = mapdiff_gitdiff::open(repo.path()).unwrap();
    let comparison = Comparison::parse(&git, None, false).unwrap();
    let changes = mapdiff_gitdiff::change_set(&git, &comparison, Options::default()).unwrap();
    let head = mapdiff_graph::build_with(repo.path(), Precise::Off, &|_| {}).unwrap();
    let report = mapdiff_graph::impact(&changes, &head, None, ImpactOptions::default());
    assert!(report.removed_not_checked);
    assert!(report.impacted.is_empty());
}
