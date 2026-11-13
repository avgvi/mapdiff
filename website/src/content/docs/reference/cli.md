---
title: Commands
description: The mapdiff command, its arguments, and the commands that print to the terminal.
---

## mapdiff

```
mapdiff [CHANGE] [--commit REV | --staged | --pr NUMBER] [--repo DIR] [--port N] [--no-open]
```

Opens the map of a repository in the browser, and runs until Ctrl-C.

| Argument | Meaning |
|---|---|
| none | Your uncommitted changes drawn on the map, or the plain map when there are none |
| `CHANGE` | `REV` (against your files as they are), `A..B`, or `A...B` (from where B branched off A) |
| `--commit REV` | One commit, against the commit before it |
| `--staged` | What is staged, against the last commit |
| `--pr NUMBER` | A GitHub pull request, fetched from `origin` |
| `--repo DIR` | A directory inside the repository. Default: the current one |
| `--port N` | Default 7878; the next free port is used when it is taken |
| `--no-open` | Print the address without opening the browser |

`mapdiff serve` is the same command, spelled out.

## In the terminal

Each of these prints what the page shows, most with `--json`.

| Command | Prints |
|---|---|
| `mapdiff diff [CHANGE]` | The change as added, removed, modified, renamed and moved scopes |
| `mapdiff diff --scope ID` | The before and after of one scope |
| `mapdiff impact [CHANGE]` | Unchanged scopes that use something the change altered |
| `mapdiff refs ID` | What uses a scope, and what it uses |
| `mapdiff wires [ID] --level LEVEL` | The wires among the children of a directory, file or type |
| `mapdiff map --level LEVEL` | What a map draws at a level: boxes and wires |
| `mapdiff scopes PATH` | The scope tree of a file or directory |
| `mapdiff source ID` | The source text of one scope |
| `mapdiff graph` | A summary of the reference graph |
| `mapdiff index` | Runs the language indexers. See [languages](../languages/) |

`ID` is a scope's ID, as the page and `mapdiff scopes` show it: `src/lib.rs::Parser::parse()`.

## Settings

| Variable | Does |
|---|---|
| `MAPDIFF_TESTS=1` | Reads tests like any other code |
| `MAPDIFF_CACHE_DIR` | Where indexes are kept |
