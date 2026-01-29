# `sysand add`

Add usage to project information.

## Usage

```sh
sysand add [OPTIONS] <IRI|--path <PATH>> [VERSION_CONSTRAINT]
```

## Description

Adds IRI and optional version constraint to list of usages in the project
information file `.project.json`. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

## Arguments

- `<IRI>`: IRI/URI/URL identifying the project to be used. See
  [`usage` field](../metadata.md#usage) for details.
- `[VERSION_CONSTRAINT]`: A constraint on the allowable versions of a used
  project. Assumes that the project uses Semantic Versioning. See
  [`versionConstraint`](../metadata.md#versionconstraint) for details

## Options

- `-p`, `--path` `<PATH>`: Path to the project to be added. Since every
  usage is identified by an IRI, `file://` URL will be used to refer to
  the project.

  Warning: using this makes the project not portable between different
  computers, as `file://` URL always contains an absolute path
- `--no-lock`: Do not automatically resolve usages (and generate lockfile)
- `--no-sync`: Do not automatically install dependencies

{{#include ./partials/resolution_opts.md}}

{{#include ./partials/global_opts.md}}
