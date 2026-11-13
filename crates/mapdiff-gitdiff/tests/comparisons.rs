//! What can be compared: commits, ranges, the index and the working tree.
//! Also whitespace handling, and that results never vary between runs.

mod common;

use common::{Diffing, TempRepo};
use mapdiff_core::{ChangeKind, ChangeSet, ChangeState};
use mapdiff_gitdiff::{Comparison, Options};

const BASE: &str = "\
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn double(value: i32) -> i32 {
    add(value, value)
}
";

fn repo_with_base(name: &str) -> TempRepo {
    let repo = TempRepo::new(name);
    repo.write("src/lib.rs", BASE);
    repo.commit("base");
    repo
}

fn changes(repo: &TempRepo, spec: Option<&str>, staged: bool, options: Options) -> ChangeSet {
    let git = repo.open();
    let comparison = Comparison::parse(&git, spec, staged).expect("comparison parses");
    let changes =
        mapdiff_gitdiff::change_set(&git, &comparison, options).expect("change set builds");
    let problems = changes.violations();
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    changes
}

fn find<'a>(changes: &'a ChangeSet, id: &str) -> &'a mapdiff_core::Change {
    changes
        .changes
        .iter()
        .find(|change| change.id() == id)
        .unwrap_or_else(|| panic!("no change for {id} in:\n{}", changes.render_text()))
}

#[test]
fn a_clean_working_tree_has_no_changes() {
    let repo = repo_with_base("clean");
    let changes = changes(&repo, None, false, Options::default());
    assert!(changes.is_empty(), "\n{}", changes.render_text());
    assert_eq!(changes.base.label, "HEAD");
    assert_eq!(changes.head.label, "working tree");
    assert!(changes.head.commit.is_none());
}

#[test]
fn uncommitted_edits_and_untracked_files_are_changes() {
    let repo = repo_with_base("worktree");
    repo.replace(
        "src/lib.rs",
        "    a + b\n",
        "    let sum = a + b;\n    sum\n",
    );
    repo.write(
        "src/extra.rs",
        "pub fn triple(value: i32) -> i32 {\n    value * 3\n}\n",
    );

    let changes = changes(&repo, None, false, Options::default());
    assert_eq!(changes.stats.files, 2, "\n{}", changes.render_text());
    assert_eq!(
        find(&changes, "src/lib.rs::add()").kinds,
        [ChangeKind::Body]
    );
    assert_eq!(find(&changes, "src/extra.rs").state, ChangeState::Added);
    assert_eq!(
        find(&changes, "src/extra.rs::triple()").state,
        ChangeState::Added
    );
    // The tracked edit as git counts it, plus every line of the new file.
    let (added, removed) = repo.git_totals(&["HEAD"]);
    assert_eq!(
        (changes.stats.added, changes.stats.removed),
        (added + 3, removed)
    );
}

#[test]
fn staged_compares_the_index_only() {
    let repo = repo_with_base("staged");
    repo.replace("src/lib.rs", "    a + b\n", "    a - b\n");
    repo.git(&["add", "src/lib.rs"]);
    // A later edit that is not staged.
    repo.replace(
        "src/lib.rs",
        "    add(value, value)\n",
        "    add(value, 0)\n",
    );

    let staged = changes(&repo, None, true, Options::default());
    assert_eq!(staged.head.label, "index");
    assert!(
        staged
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::add()")
    );
    assert!(
        !staged
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::double()")
    );

    let all = changes(&repo, None, false, Options::default());
    assert!(
        all.changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::double()")
    );
}

#[test]
fn two_dots_compare_commits_and_three_dots_compare_from_the_branch_point() {
    let repo = repo_with_base("ranges");
    repo.git(&["checkout", "--quiet", "-b", "feature"]);
    repo.replace("src/lib.rs", "    a + b\n", "    a - b\n");
    repo.commit("feature work");
    repo.git(&["checkout", "--quiet", "main"]);
    repo.replace(
        "src/lib.rs",
        "    add(value, value)\n",
        "    add(value, 0)\n",
    );
    repo.commit("main work");

    // main..feature sees both edits: main's is undone going to feature.
    let two = changes(&repo, Some("main..feature"), false, Options::default());
    assert!(
        two.changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::add()")
    );
    assert!(
        two.changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::double()")
    );

    // main...feature sees only what the branch did.
    let three = changes(&repo, Some("main...feature"), false, Options::default());
    assert!(
        three
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::add()")
    );
    assert!(
        !three
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::double()")
    );
    assert!(three.head.commit.is_some());
}

#[test]
fn whitespace_only_changes_can_be_left_out() {
    let repo = repo_with_base("whitespace");
    repo.replace("src/lib.rs", "    a + b\n", "        a   +   b\n");
    repo.replace(
        "src/lib.rs",
        "    add(value, value)\n",
        "    add(value, 0)\n",
    );

    let shown = changes(&repo, None, false, Options::default());
    assert_eq!(
        find(&shown, "src/lib.rs::add()").kinds,
        [ChangeKind::Formatting]
    );

    let hidden = changes(
        &repo,
        None,
        false,
        Options {
            ignore_whitespace: true,
        },
    );
    assert!(
        !hidden
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::add()"),
        "\n{}",
        hidden.render_text()
    );
    assert!(
        hidden
            .changes
            .iter()
            .any(|change| change.id() == "src/lib.rs::double()")
    );
    assert_eq!((hidden.stats.added, hidden.stats.removed), (1, 1));
}

#[test]
fn an_access_change_alone_is_reported() {
    let repo = TempRepo::new("access");
    repo.write("pkg/shapes.py", "def area(shape):\n    return shape.width * shape.height\n\n\ndef perimeter(shape):\n    return 2 * (shape.width + shape.height)\n");
    repo.commit("base");
    // Listing only `area` in `__all__` makes `perimeter` restricted,
    // without touching its text.
    repo.replace(
        "pkg/shapes.py",
        "def area",
        "__all__ = [\"area\"]\n\n\ndef area",
    );

    let changes = changes(&repo, None, false, Options::default());
    let perimeter = find(&changes, "pkg/shapes.py::perimeter()");
    assert_eq!(perimeter.kinds, [ChangeKind::Access]);
    assert!(perimeter.own.is_zero());
}

#[test]
fn files_git_cannot_diff_as_text_are_listed_without_lines() {
    let repo = repo_with_base("binary");
    repo.write("logo.bin", "\u{0}\u{1}\u{2}binary");
    repo.commit("add a binary file");
    std::fs::write(repo.path().join("logo.bin"), [0u8, 9, 9, 9, 0]).unwrap();
    repo.commit("change it");

    let changes = repo.last_commit();
    assert_eq!(changes.stats.files, 1);
    assert_eq!((changes.stats.added, changes.stats.removed), (0, 0));
}

#[test]
fn the_same_comparison_always_gives_the_same_change_set() {
    let repo = repo_with_base("stable");
    repo.replace("src/lib.rs", "    a + b\n", "    a - b\n");
    repo.write(
        "src/extra.rs",
        "pub fn triple(value: i32) -> i32 {\n    value * 3\n}\n",
    );
    let first = changes(&repo, None, false, Options::default());
    for _ in 0..5 {
        assert_eq!(first, changes(&repo, None, false, Options::default()));
    }
}

#[test]
fn a_bad_revision_is_an_error_not_a_panic() {
    let repo = repo_with_base("bad-revision");
    let git = repo.open();
    let error = Comparison::parse(&git, Some("no-such-branch"), false).unwrap_err();
    assert!(error.to_string().contains("no-such-branch"), "{error}");
    assert!(
        Comparison::commit(&git, "HEAD").is_err(),
        "the first commit has no parent"
    );
}

/// One change set as full JSON, to pin the serialised shape. Commit hashes
/// are blanked so the snapshot does not depend on them.
#[test]
fn json_shape() {
    let repo = repo_with_base("json");
    repo.replace(
        "src/lib.rs",
        "pub fn add(a: i32, b: i32)",
        "pub fn add(a: i64, b: i32)",
    );
    repo.commit("widen a parameter");
    let mut changes = repo.last_commit();
    changes.base.commit = None;
    changes.head.commit = None;
    insta::assert_json_snapshot!(changes);
}
