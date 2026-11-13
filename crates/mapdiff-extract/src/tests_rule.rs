//! What counts as test code, for every language, by where a file is and
//! what it is called.
//!
//! Tests are left out when a whole repository is read (`extract_repo`): a
//! test file is never opened. A change to a test is still seen, since a
//! change set reads the changed files themselves.

/// Set to `1` to read tests like any other code.
pub const KEEP_TESTS: &str = "MAPDIFF_TESTS";

/// Directory names that hold tests in the supported languages.
const TEST_DIRECTORIES: [&str; 5] = ["tests", "test", "__tests__", "spec", "specs"];

/// True when a repository-relative path (with `/`) is a test by its place
/// or its name.
///
/// - any language: a file under `tests/`, `test/`, `__tests__/`, `spec/`
/// - JavaScript and TypeScript: `name.test.ts`, `name.spec.js`
/// - Python: `test_name.py`, `name_test.py`, `conftest.py`
/// - Rust: `name_test.rs`, `name_tests.rs`, `tests.rs`
pub fn is_test_path(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    if parts.iter().any(|part| TEST_DIRECTORIES.contains(part)) {
        return true;
    }
    if name.contains(".test.") || name.contains(".spec.") {
        return true;
    }
    if let Some(stem) = name.strip_suffix(".py") {
        return stem.starts_with("test_") || stem.ends_with("_test") || stem == "conftest";
    }
    if let Some(stem) = name.strip_suffix(".rs") {
        return stem.ends_with("_test") || stem.ends_with("_tests") || stem == "tests";
    }
    false
}

/// True when the reader asked for tests to be kept.
pub fn keeping_tests() -> bool {
    std::env::var(KEEP_TESTS).is_ok_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::is_test_path;

    #[test]
    fn tests_are_told_by_place_and_by_name() {
        for path in [
            "tests/integration.rs",
            "crates/ignore/tests/gitignore.rs",
            "src/tests/checker_tests.rs",
            "src/checker_tests.rs",
            "test/app.listen.js",
            "src/__tests__/store.ts",
            "src/store.test.tsx",
            "lib/view.spec.js",
            "tests/client/test_auth.py",
            "httpx/test_utils.py",
            "pkg/models_test.py",
            "conftest.py",
        ] {
            assert!(is_test_path(path), "{path} is a test");
        }
        for path in [
            "src/main.rs",
            "src/testing_utils.rs",
            "src/contest.rs",
            "httpx/_client.py",
            "src/latest.ts",
            "lib/attest/index.js",
            "docs/tests.md.txt",
        ] {
            assert!(!is_test_path(path), "{path} is not a test");
        }
    }
}
