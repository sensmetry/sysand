# `sysand info license`

Get or set the license of the project

## Usage

```sh
sysand info license [OPTIONS]
```

## Description

Prints the license of the given project. With `--set` or `--clear`, modifies the license.
`licence` is an accepted alias for this subcommand.

By default the license must be a valid [SPDX license expression][spdx].
Pass `--no-spdx` alongside `--set` to allow an arbitrary string.
See [Project metadata](../../metadata.md#license) for guidance on license format.

[spdx]: https://spdx.github.io/spdx-spec/v3.0.1/annexes/spdx-license-expressions/

## Options

- `--set <LICENSE>`: Set the license in the form of an SPDX license identifier
- `--no-spdx`: Don't require license to be an SPDX expression
- `--clear`: Remove the project's license

{{#include ../partials/global_opts.md}}
