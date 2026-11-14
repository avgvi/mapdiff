# Changelog

## 0.2.0 - 2026-11-13

- **`mapdiff --index`** runs the language indexers in the background. The map opens straight away and gains references when they finish; after files change, they run again the next time the page is loaded.
- **The bar above the map says how references are found:** by name, indexing, or by type, with an **Index** button and a note when an indexer is not installed.
- **`mapdiff list`** shows every running server: its repository, address, the change it last drew, and how long it has been up.
- **`mapdiff stop`** stops the server of the repository you are in, and `--all` stops every one.
- **`mapdiff clean`** stops every server and removes everything mapdiff has written: review marks, fetched pull requests, temporary copies of files and the cache folder.
- A server stopped with `kill` now removes its temporary files, as one stopped with Ctrl-C does.

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
