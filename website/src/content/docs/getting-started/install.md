---
title: Install
description: Build mapdiff from source and add the optional language indexers.
---

`mapdiff` is built from source. You need Rust (1.88 or later) and Node (22 or later).

```bash
git clone https://github.com/avgvi/mapdiff && cd mapdiff
cd ui && npm ci && npm run build && cd ..
cargo install --path crates/mapdiff-cli
```

The second line builds the page. It has to come before the third, because the page is put inside the `mapdiff` binary: once installed, the binary needs nothing else and runs from any folder.

Check it:

```bash
mapdiff --version
```

## Optional: references by type

Out of the box mapdiff finds what uses what by name. To also find methods called on a value (`client.send()`), install the indexer for your language. mapdiff uses whichever it finds.

| Language | Indexer | Install |
|---|---|---|
| Rust | rust-analyzer | `rustup component add rust-analyzer` |
| TypeScript, JavaScript | scip-typescript | `npm install -g @sourcegraph/scip-typescript` |
| Python | scip-python | `npm install -g @sourcegraph/scip-python` |

Then run `mapdiff index` once in a repository. See [languages and references](../../reference/languages/).

## Uninstall

```bash
cargo uninstall mapdiff
```

mapdiff keeps indexes in your cache folder (`~/Library/Caches/mapdiff` on macOS, `~/.cache/mapdiff` on Linux) and review marks in each repository's `.git/mapdiff/`.

Next: the [quick start](../quick-start/).
