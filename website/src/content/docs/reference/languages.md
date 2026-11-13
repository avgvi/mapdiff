---
title: Languages and references
description: What mapdiff reads in each language, how it finds references, and how tests are recognised.
---

## What is read

| Language | Files | Scopes |
|---|---|---|
| Rust | `.rs` | modules, structs, enums, traits, impls, functions, fields, variants, constants, macros |
| Python | `.py` | classes, functions, methods, fields, constants |
| TypeScript | `.ts`, `.tsx` | classes, interfaces, enums, type aliases, functions, methods, fields |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` | classes, functions, methods, fields |

Other files are on the map as documents: one box each, with no inside.

## What "used by" can see

There are two ways mapdiff finds what uses what.

**By name**, always on. It follows imports and resolves the names a file uses: a function called, a type used, a class extended. It takes well under a second and is almost never wrong about what it finds. It cannot find a method called on a value (`client.send()`), because that needs the type of `client`.

**By type**, with an index. The language's own indexer (rust-analyzer, scip-typescript, scip-python) adds what only types can tell. It can take minutes on a large project, and its result is kept until a file changes. There are three ways to make one:

| How | What happens |
|---|---|
| `mapdiff --index` | The map opens straight away, the indexers run in the background, and the map gains references when they finish. After you change files, they run again the next time the page is loaded |
| **Index**, in the bar above the map | The same, started from the page |
| `mapdiff index` | Runs them in the terminal and waits. For a script, or to have an index ready before opening the map |

The bar above the map says which you are looking at: **References: by name**, **indexing…** or **by type**. If an indexer is not installed, it says so there, with how to get it.

Measured on four open-source repositories:

| Repository | Found by name, of what the indexer finds | Agreed with the indexer |
|---|---|---|
| ripgrep (Rust) | 52% | 99.7% |
| httpx (Python) | 58% | 99.7% |
| zustand (TypeScript) | 58% | 99.9% |
| express (JavaScript) | 76% | 100% |

So without an index, "Used by" is a floor: what it lists is right, and it may be missing method calls.

A comparison of two commits uses names only, since an index covers the files as they are now.

## Tests

Tests are not read: not parsed, not on the map, not counted in any wire.

| What | Recognised by |
|---|---|
| A test file | Under `tests/`, `test/`, `__tests__/` or `spec/`; `name.test.ts`, `name.spec.js`; `test_name.py`, `name_test.py`, `conftest.py`; `name_test.rs`, `name_tests.rs`, `tests.rs` |
| Test code in a source file | A Rust `#[test]` function or `#[cfg(test)]` module; a Python `test_` function or `Test` class |

A changed test still shows in a review, listed after the code. `MAPDIFF_TESTS=1` reads tests like any other code.
