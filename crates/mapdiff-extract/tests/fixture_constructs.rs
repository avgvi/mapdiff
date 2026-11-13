//! Guards the hand-made fixtures in `fixtures/<language>/`.
//!
//! Each fixture tags its constructs with `construct: <name>` comments. Each
//! language also has a list of required constructs in
//! `fixtures/constructs/<language>.txt`. This test fails when a required
//! construct has no marker, or when a marker is not in the list (usually a
//! typo, or a new construct that still needs listing).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const LANGUAGES: [(&str, &[&str]); 4] = [
    ("rust", &["rs"]),
    ("python", &["py"]),
    ("typescript", &["ts", "tsx"]),
    ("javascript", &["js", "mjs", "cjs", "jsx"]),
];

const MARKER: &str = "construct:";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn source_files(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.is_dir() {
            if name != "target" && name != "node_modules" {
                source_files(&path, extensions, out);
            }
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| extensions.contains(&ext))
        {
            out.push(path);
        }
    }
}

/// Names after `construct:` on a line, split on commas.
fn markers_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let Some(index) = line.find(MARKER) else {
            continue;
        };
        for part in line[index + MARKER.len()..].split(',') {
            let name: String = part
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .collect();
            if !name.is_empty() {
                found.push(name);
            }
        }
    }
    found
}

fn required(language: &str) -> BTreeSet<String> {
    let path = fixtures_dir()
        .join("constructs")
        .join(format!("{language}.txt"));
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_required_construct_has_a_marker() {
    let mut problems = Vec::new();

    for (language, extensions) in LANGUAGES {
        let mut files = Vec::new();
        source_files(&fixtures_dir().join(language), extensions, &mut files);
        assert!(!files.is_empty(), "no fixture files for {language}");

        let mut found = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file).expect("fixture is UTF-8");
            found.extend(markers_in(&text));
        }

        let required = required(language);
        for missing in required.difference(&found) {
            problems.push(format!(
                "{language}: required construct `{missing}` has no marker"
            ));
        }
        for unlisted in found.difference(&required) {
            problems.push(format!(
                "{language}: marker `{unlisted}` is not in fixtures/constructs/{language}.txt"
            ));
        }
    }

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn marker_parsing_handles_lists_and_trailing_text() {
    assert_eq!(
        markers_in("// construct: fn, access-pub"),
        ["fn", "access-pub"]
    );
    assert_eq!(markers_in("# construct: class (see above)"), ["class"]);
    assert!(markers_in("no marker here").is_empty());
}
