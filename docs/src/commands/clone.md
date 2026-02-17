# `sysand clone`

Clone a project to a specified directory. Equivalent to manually downloading,
extracting the project to the directory and running `sysand sync`.

## Usage

```sh
sysand clone [OPTIONS] <LOCATOR|--iri <IRI>|--path <PATH>>
```

## Description

Allows the user to quickly pull and open a library or an example from a project
index. Acts similar to `git clone` -- it needs an empty repository to write the
contents to.

## Arguments

- `[LOCATOR]`: Clone the project from a given locator, trying to parse it as an
  IRI/URI/URL and otherwise falling back to using it as a path

## Options

- `-i`, `--iri <IRI>`: IRI/URI/URL identifying the project to be cloned
  [aliases: `--uri`, `--url`]
- `-s`, `--path <PATH>`: Path to clone the project from. If version is also
  given, verifies that the project has the given version
- `-t`, `--target <TARGET>`: Path to clone the project into. If already exists,
  must be an empty directory. Defaults to current directory
- `-V`, `--version <VERSION>`: Version of the project to clone. Defaults to the
  latest version according to SemVer 2.0
- `--no-deps`: Don't resolve or install dependencies

{{#include ./partials/resolution_opts.md}}

{{#include ./partials/global_opts.md}}
