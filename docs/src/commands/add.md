# `sysand add`

Add usage to project information

## Usage

```sh
sysand add [OPTIONS] <IRI> [VERSION_CONSTRAINT]
```

## Description

Adds IRI and optional version constraint to list of usages in the project
information file `.project.json`. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

## Arguments

- `<IRI>`: IRI identifying the project to be used
- `[VERSION_CONSTRAINT]`: A constraint on the allowable versions of a used project
                          Assumes that the project uses Semantic Versioning
                          See [`versionConstraint` docs](../metadata.md#versionconstraint) for details

## Options

- `--no-lock`: Do not automatically resolve usages (and generate lockfile)
- `--no-sync`: Do not automatically install dependencies

{{#include ./partials/dependency_opts.md}}

{{#include ./partials/global_opts.md}}
