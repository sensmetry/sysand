# `sysand index remove`

Remove a project or a specific version of a project from a local sysand index.

## Usage

```sh
sysand index remove [OPTIONS] <--version <VERSION>|--project> <IRI>
```

## Description

Remove a project or a specific version of a project from a local sysand index.
This breaks the existing lockfiles which use the to-be-removed project or version.
Instead it is recommended to yank a specific version and release a new fixed version.
Project or version removal cannot be undone.

## Arguments

- `<IRI>`: Project identifier

## Options

- `--version <VERSION>`: Version to remove
- `--project`: Remove the whole project
- `--index-root <INDEX_ROOT>`: Path to the index directory. If not provided, current working directory is used

```{include} ../partials/global_opts.md

```
