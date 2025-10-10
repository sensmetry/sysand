# `sysand env sources`

List source files for an installed project and (optionally) its dependencies

## Usage

```sh
sysand env sources [OPTIONS] <IRI> [VERSION]
```

## Description

Prints the paths to the source files (separated by newlines) for an installed
project and (optionally) its dependencies. Is intended to be machine readable.

By default sources for standard libraries are not included, as they are
typically shipped with your language implementation.

## Arguments

- `<IRI>`: IRI of the (already installed) project for which to enumerate source files
- `[VERSION]`: Version of project to list sources for

## Options

- `--no-deps`: Do not include sources for dependencies
- `--include-std`: Include (installed) KerML/SysML standard libraries
