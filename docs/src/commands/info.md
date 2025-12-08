# `sysand info`

Resolve and describe current project or one at at a specified path or IRI/URL

## Usage

```sh
sysand info [OPTIONS]
sysand info [OPTIONS] <COMMAND>
```

## Description

Prints out the information contained in the `.project.json` file for the specified
project, defaulting to current project if no project is specified. Optionally an
extra command can be given to get or set values in `.project.json` and `.meta.json`.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Options

- `--path <PATH>`: Use the project at the given path instead of the current project
- `--iri <PATH>`: Use the project with the given IRI/URI/URL instead of the
  current project
- `--auto-location <AUTO_LOCATION>`: Use the project with the given location, trying
  to parse it as an IRI/URI/URL and otherwise falling back to a local path
- `--no-normalise`: Do not try to normalise the IRI/URI when resolving

{{#include ./partials/dependency_opts.md}}

{{#include ./partials/global_opts.md}}
