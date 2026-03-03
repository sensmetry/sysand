# `sysand info version`

Get or set the version of the project

## Usage

```sh
sysand info version [OPTIONS]
```

## Description

Prints the version of the current project. With `--set`, updates the version.

By default the version must conform to [Semantic Versioning 2.0][semver].
Pass `--no-semver` alongside `--set` to allow an arbitrary version string.

The `version` field is required and cannot be cleared.

[semver]: https://semver.org/

## Options

- `--set <VERSION>`: Set the version (SemVer 2.0 by default)
- `--no-semver`: Allow a non-SemVer version string (requires `--set`)

{{#include ../partials/global_opts.md}}
