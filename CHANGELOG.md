# Changelog

## 0.1.0 - 2026-11-13

The first version.

- **Scopes** for Rust, Python, TypeScript and JavaScript: a repository as directories, files, types, functions and fields, each with an ID.
- **Changes by scope:** added, removed, modified, renamed and moved, with what kind of change it was (signature, body, comments, formatting, access). Line counts equal git's.
- **A reference graph:** by name out of the box, and by type after `mapdiff index` (rust-analyzer, scip-typescript, scip-python).
- **A reviewer page** (`mapdiff serve`): the change as a tree of scopes with before and after side by side, filters, keyboard navigation and review marks.
- **A map** of the repository at four levels, drawing only what is on screen, with search, selection, paths to callers and dependencies, and a trace view.
- **A change on the map:** colours and counts, a list in reading order, before and after under the map, affected and neighbouring code, the map cut down to the change, GitHub pull requests by number.
- **Tests are left out** of the map and the graph, and are not read. A changed test is still listed in a review.
- **One binary:** the page is built into `mapdiff`.
