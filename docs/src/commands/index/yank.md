# `sysand index yank`

Yank a project version from a local index.

## Usage

```sh
sysand index yank [OPTIONS] --version <VERSION> <IRI>
```

## Description

Yank a project version from a local index. The yanked version will still be available
and used to sync from an existing lockfile, but new lockfiles will not use it.
A yanked version cannot be un-yanked.

## Arguments

- `<IRI>` Project identifier

## Options

`--version <VERSION>`: Version to yank
`--index-root <INDEX_ROOT>`: Path to the index directory. If not provided, current working directory is used

{{#include ../partials/global_opts.md}}
