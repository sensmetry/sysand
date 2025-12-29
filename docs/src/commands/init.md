# `sysand init`

Create new project in current directory

## Usage

```sh
sysand init [OPTIONS] [PATH]
```

## Description

Create new project in current directory, i.e. create `.project.json` and
`.meta.json` files.

## Arguments

- `[PATH]`: The path to use for the project. Defaults to current directory

## Options

- `--name <NAME>`: The name of the project. Defaults to the directory name
- `--version <VERSION>`: Set the version in SemVer 2.0 format. Defaults to `0.0.1`
- `--no-semver`: Don't require version to conform to SemVer
- `--license <LICENSE>`: Set the license in the form of an SPDX license identifier.
  Defaults to omitting the license field
- `--no-spdx`: Don't require license to be an SPDX expression

{{#include ./partials/global_opts.md}}
