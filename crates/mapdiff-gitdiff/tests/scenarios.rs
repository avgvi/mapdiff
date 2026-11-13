//! Known edits, one commit each, in a throwaway repository per language.
//! The snapshot of each language lists what the differ reports for every
//! edit. Each change set is also checked against git's own line totals.

mod common;

use common::{Diffing, TempRepo};

/// One edit: a title for the snapshot and what to do to the repository.
type Step<'a> = (&'a str, &'a dyn Fn(&TempRepo));

fn run(name: &str, files: &[(&str, &str)], steps: &[Step<'_>]) -> String {
    let repo = TempRepo::new(name);
    for (path, content) in files {
        repo.write(path, content);
    }
    repo.commit("base");

    let mut report = String::new();
    for (title, edit) in steps {
        edit(&repo);
        repo.commit(title);
        report.push_str(&format!(
            "## {title}\n{}\n",
            repo.last_commit().render_text()
        ));
    }
    report
}

const RUST_SHAPES: &str = "\
use std::fmt;

/// A point on a plane.
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

pub fn midpoint(a: &Point, b: &Point) -> Point {
    let x = (a.x + b.x) / 2.0;
    let y = (a.y + b.y) / 2.0;
    Point::new(x, y)
}

fn helper(values: &[f64]) -> f64 {
    let mut total = 0.0;
    for value in values {
        total += value;
    }
    total / values.len() as f64
}
";

const RUST_UTIL: &str = "\
pub fn clamp(value: f64, low: f64, high: f64) -> f64 {
    if value < low {
        return low;
    }
    if value > high {
        return high;
    }
    value
}
";

const RUST_MIDPOINT: &str = "\
pub fn midpoint(a: &Point, b: &Point, scale: f64) -> Point {
    let x = (a.x + b.x) / 2.0 * scale;
    let y = (a.y + b.y) / 2.0;
    Point::new(x, y)
}
";

#[test]
fn rust_scenarios() {
    let report = run(
        "rust",
        &[("src/shapes.rs", RUST_SHAPES), ("src/util.rs", RUST_UTIL)],
        &[
            ("edit a body", &|repo| {
                repo.replace(
                    "src/shapes.rs",
                    "        (dx * dx + dy * dy).sqrt()\n",
                    "        let squared = dx * dx + dy * dy;\n        squared.sqrt()\n",
                );
            }),
            ("change a signature", &|repo| {
                repo.replace(
                    "src/shapes.rs",
                    "pub fn midpoint(a: &Point, b: &Point) -> Point {\n    let x = (a.x + b.x) / 2.0;\n",
                    "pub fn midpoint(a: &Point, b: &Point, scale: f64) -> Point {\n    let x = (a.x + b.x) / 2.0 * scale;\n",
                );
            }),
            ("rename a function", &|repo| {
                repo.replace("src/shapes.rs", "fn helper(values", "fn average(values");
            }),
            ("move a function to another file", &|repo| {
                repo.move_text("src/shapes.rs", "src/util.rs", RUST_MIDPOINT);
            }),
            ("reformat only", &|repo| {
                repo.replace(
                    "src/util.rs",
                    "    if value < low {\n        return low;\n    }\n",
                    "    if value < low { return low; }\n",
                );
            }),
            ("edit a comment only", &|repo| {
                repo.replace(
                    "src/shapes.rs",
                    "/// A point on a plane.",
                    "/// A point in two dimensions.",
                );
            }),
            ("add a field", &|repo| {
                repo.replace(
                    "src/shapes.rs",
                    "    pub y: f64,\n}",
                    "    pub y: f64,\n    pub label: String,\n}",
                );
            }),
            ("change access", &|repo| {
                repo.replace(
                    "src/shapes.rs",
                    "fn average(values",
                    "pub fn average(values",
                );
            }),
            ("delete a function", &|repo| {
                let content = repo.read("src/shapes.rs");
                let start = content.find("pub fn average").expect("average exists");
                repo.write("src/shapes.rs", content[..start].trim_end_matches('\n'));
                repo.replace("src/shapes.rs", "}\n}", "}\n}\n");
            }),
            ("rename a file", &|repo| {
                repo.git(&["mv", "src/util.rs", "src/tools.rs"]);
            }),
            ("move a function and edit it", &|repo| {
                let content = repo.read("src/tools.rs");
                let start = content.find("pub fn clamp").expect("clamp exists");
                let end = content[start..].find("\n}\n").expect("clamp ends") + start + 3;
                let function = content[start..end].to_owned();
                repo.move_text("src/tools.rs", "src/shapes.rs", &function);
                repo.replace(
                    "src/shapes.rs",
                    "    if value > high {\n        return high;\n    }\n",
                    "    if value >= high {\n        return high;\n    }\n",
                );
            }),
        ],
    );
    insta::assert_snapshot!(report);
}

const PYTHON_SHAPES: &str = "\
import math


# A point on a plane.
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def distance(self, other):
        dx = self.x - other.x
        dy = self.y - other.y
        return math.sqrt(dx * dx + dy * dy)


def midpoint(a, b):
    x = (a.x + b.x) / 2
    y = (a.y + b.y) / 2
    return Point(x, y)


def _helper(values):
    total = 0
    for value in values:
        total += value
    return total / len(values)
";

const PYTHON_UTIL: &str = "\
def clamp(value, low, high):
    if value < low:
        return low
    if value > high:
        return high
    return value
";

const PYTHON_MIDPOINT: &str = "\
def midpoint(a, b, scale):
    x = (a.x + b.x) / 2 * scale
    y = (a.y + b.y) / 2
    return Point(x, y)
";

#[test]
fn python_scenarios() {
    let report = run(
        "python",
        &[
            ("pkg/shapes.py", PYTHON_SHAPES),
            ("pkg/util.py", PYTHON_UTIL),
        ],
        &[
            ("edit a body", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "        return math.sqrt(dx * dx + dy * dy)\n",
                    "        squared = dx * dx + dy * dy\n        return math.sqrt(squared)\n",
                );
            }),
            ("change a signature", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "def midpoint(a, b):\n    x = (a.x + b.x) / 2\n",
                    "def midpoint(a, b, scale):\n    x = (a.x + b.x) / 2 * scale\n",
                );
            }),
            ("rename a function", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "def _helper(values)",
                    "def _average(values)",
                );
            }),
            ("move a function to another file", &|repo| {
                repo.replace("pkg/shapes.py", &format!("{PYTHON_MIDPOINT}\n\n"), "");
                let mut util = repo.read("pkg/util.py");
                util.push_str(&format!("\n\n{PYTHON_MIDPOINT}"));
                repo.write("pkg/util.py", &util);
            }),
            ("reformat only", &|repo| {
                repo.replace(
                    "pkg/util.py",
                    "def clamp(value, low, high):",
                    "def clamp(value,  low,  high):",
                );
            }),
            ("edit a comment only", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "# A point on a plane.",
                    "# A point in two dimensions.",
                );
            }),
            ("add a field", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "        self.y = y\n",
                    "        self.y = y\n        self.label = \"\"\n",
                );
            }),
            ("change access", &|repo| {
                repo.replace(
                    "pkg/shapes.py",
                    "import math\n",
                    "import math\n\n__all__ = [\"Point\"]\n",
                );
            }),
            ("delete a function", &|repo| {
                let content = repo.read("pkg/shapes.py");
                let start = content.find("def _average").expect("_average exists");
                repo.write(
                    "pkg/shapes.py",
                    &format!("{}\n", content[..start].trim_end_matches('\n')),
                );
            }),
            ("rename a file", &|repo| {
                repo.git(&["mv", "pkg/util.py", "pkg/tools.py"]);
            }),
            ("move a function and edit it", &|repo| {
                repo.replace(
                    "pkg/tools.py",
                    &PYTHON_UTIL.replace("value, low, high", "value,  low,  high"),
                    "",
                );
                let mut shapes = repo.read("pkg/shapes.py");
                shapes.push_str(&format!(
                    "\n\n{}",
                    PYTHON_UTIL.replace("value > high", "value >= high")
                ));
                repo.write("pkg/shapes.py", &shapes);
            }),
        ],
    );
    insta::assert_snapshot!(report);
}

const TS_SHAPES: &str = "\
import { clamp } from \"./util\";

/** A point on a plane. */
export class Point {
  label = \"\";

  constructor(
    public x: number,
    public y: number,
  ) {}

  distance(other: Point): number {
    const dx = this.x - other.x;
    const dy = this.y - other.y;
    return Math.sqrt(dx * dx + dy * dy);
  }
}

export function midpoint(a: Point, b: Point): Point {
  const x = (a.x + b.x) / 2;
  const y = (a.y + b.y) / 2;
  return new Point(x, y);
}

function helper(values: number[]): number {
  let total = 0;
  for (const value of values) {
    total += clamp(value, 0, 100);
  }
  return total / values.length;
}
";

const TS_UTIL: &str = "\
export function clamp(value: number, low: number, high: number): number {
  if (value < low) {
    return low;
  }
  if (value > high) {
    return high;
  }
  return value;
}
";

const TS_MIDPOINT: &str = "\
export function midpoint(a: Point, b: Point, scale: number): Point {
  const x = ((a.x + b.x) / 2) * scale;
  const y = (a.y + b.y) / 2;
  return new Point(x, y);
}
";

#[test]
fn typescript_scenarios() {
    let report = run(
        "typescript",
        &[("src/shapes.ts", TS_SHAPES), ("src/util.ts", TS_UTIL)],
        &[
            ("edit a body", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "    return Math.sqrt(dx * dx + dy * dy);\n",
                    "    const squared = dx * dx + dy * dy;\n    return Math.sqrt(squared);\n",
                );
            }),
            ("change a signature", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "export function midpoint(a: Point, b: Point): Point {\n  const x = (a.x + b.x) / 2;\n",
                    "export function midpoint(a: Point, b: Point, scale: number): Point {\n  const x = ((a.x + b.x) / 2) * scale;\n",
                );
            }),
            ("rename a function", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "function helper(values",
                    "function average(values",
                );
            }),
            ("move a function to another file", &|repo| {
                repo.move_text("src/shapes.ts", "src/util.ts", TS_MIDPOINT);
            }),
            ("reformat only", &|repo| {
                repo.replace(
                    "src/util.ts",
                    "  if (value < low) {\n    return low;\n  }\n",
                    "  if (value < low) { return low; }\n",
                );
            }),
            ("edit a comment only", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "/** A point on a plane. */",
                    "/** A point in two dimensions. */",
                );
            }),
            ("add a field", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "  label = \"\";\n",
                    "  label = \"\";\n  visible = true;\n",
                );
            }),
            ("change access", &|repo| {
                repo.replace(
                    "src/shapes.ts",
                    "function average(values",
                    "export function average(values",
                );
            }),
            ("delete a function", &|repo| {
                let content = repo.read("src/shapes.ts");
                let start = content
                    .find("export function average")
                    .expect("average exists");
                repo.write(
                    "src/shapes.ts",
                    &format!("{}\n", content[..start].trim_end_matches('\n')),
                );
            }),
            ("rename a file", &|repo| {
                repo.git(&["mv", "src/util.ts", "src/tools.ts"]);
            }),
            ("move a function and edit it", &|repo| {
                let content = repo.read("src/tools.ts");
                let start = content.find("export function clamp").expect("clamp exists");
                let end = content[start..].find("\n}\n").expect("clamp ends") + start + 3;
                let function = content[start..end].to_owned();
                repo.move_text("src/tools.ts", "src/shapes.ts", &function);
                repo.replace(
                    "src/shapes.ts",
                    "  if (value > high) {",
                    "  if (value >= high) {",
                );
            }),
        ],
    );
    insta::assert_snapshot!(report);
}

const JS_SHAPES: &str = "\
'use strict';

const { clamp } = require('./util');

/** A point on a plane. */
class Point {
  label = '';

  constructor(x, y) {
    this.x = x;
    this.y = y;
  }

  distance(other) {
    const dx = this.x - other.x;
    const dy = this.y - other.y;
    return Math.sqrt(dx * dx + dy * dy);
  }
}

function midpoint(a, b) {
  const x = (a.x + b.x) / 2;
  const y = (a.y + b.y) / 2;
  return new Point(x, y);
}

function helper(values) {
  let total = 0;
  for (const value of values) {
    total += clamp(value, 0, 100);
  }
  return total / values.length;
}

module.exports = { Point, midpoint };
";

const JS_UTIL: &str = "\
'use strict';

function clamp(value, low, high) {
  if (value < low) {
    return low;
  }
  if (value > high) {
    return high;
  }
  return value;
}

exports.clamp = clamp;
";

const JS_MIDPOINT: &str = "\
function midpoint(a, b, scale) {
  const x = ((a.x + b.x) / 2) * scale;
  const y = (a.y + b.y) / 2;
  return new Point(x, y);
}
";

#[test]
fn javascript_scenarios() {
    let report = run(
        "javascript",
        &[("lib/shapes.js", JS_SHAPES), ("lib/util.js", JS_UTIL)],
        &[
            ("edit a body", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "    return Math.sqrt(dx * dx + dy * dy);\n",
                    "    const squared = dx * dx + dy * dy;\n    return Math.sqrt(squared);\n",
                );
            }),
            ("change a signature", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "function midpoint(a, b) {\n  const x = (a.x + b.x) / 2;\n",
                    "function midpoint(a, b, scale) {\n  const x = ((a.x + b.x) / 2) * scale;\n",
                );
            }),
            ("rename a function", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "function helper(values",
                    "function average(values",
                );
            }),
            ("move a function to another file", &|repo| {
                repo.move_text("lib/shapes.js", "lib/util.js", JS_MIDPOINT);
            }),
            ("reformat only", &|repo| {
                repo.replace(
                    "lib/util.js",
                    "  if (value < low) {\n    return low;\n  }\n",
                    "  if (value < low) { return low; }\n",
                );
            }),
            ("edit a comment only", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "/** A point on a plane. */",
                    "/** A point in two dimensions. */",
                );
            }),
            ("add a field", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "  label = '';\n",
                    "  label = '';\n  visible = true;\n",
                );
            }),
            ("change access", &|repo| {
                repo.replace(
                    "lib/shapes.js",
                    "module.exports = { Point, midpoint };",
                    "module.exports = { Point };\nmodule.exports.average = average;",
                );
            }),
            ("delete a function", &|repo| {
                let content = repo.read("lib/shapes.js");
                let start = content.find("function average").expect("average exists");
                let end = content.find("module.exports").expect("exports exist");
                repo.write(
                    "lib/shapes.js",
                    &format!("{}{}", &content[..start], &content[end..]),
                );
                repo.replace("lib/shapes.js", "\nmodule.exports.average = average;", "");
            }),
            ("rename a file", &|repo| {
                repo.git(&["mv", "lib/util.js", "lib/tools.js"]);
            }),
            ("move a function and edit it", &|repo| {
                let content = repo.read("lib/tools.js");
                let start = content.find("function clamp").expect("clamp exists");
                let end = content[start..].find("\n}\n").expect("clamp ends") + start + 3;
                let function = content[start..end].to_owned();
                repo.move_text("lib/tools.js", "lib/shapes.js", &function);
                repo.replace(
                    "lib/shapes.js",
                    "  if (value > high) {",
                    "  if (value >= high) {",
                );
            }),
        ],
    );
    insta::assert_snapshot!(report);
}
