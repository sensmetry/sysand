# `sysand info version`

Get or set the version of the project

## Usage

```sh
sysand info version [OPTIONS]
```

## Description

Prints the version of the given project. For index projects, prints all
available versions of the project. With `--set`, updates the version.

By default the version must conform to [Semantic Versioning 2.0][semver].
Pass `--no-semver` alongside `--set` to allow an arbitrary version string.

The `version` field is required and cannot be cleared.

[semver]: https://semver.org/

## Options

- `--set <VERSION>`: Set the version in SemVer 2.0 format
- `--no-semver`: Don't require version to conform to Semantic Versioning

{{#include ../partials/global_opts.md}}
