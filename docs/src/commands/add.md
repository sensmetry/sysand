# `sysand add`

Add usage to project information

## Usage

```sh
sysand add [OPTIONS] <IRI> [VERSIONS_CONSTRAINT]
```

## Description

Adds IRI and optional version constraint to list of usages in the project
information file `.project.json`. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

## Arguments

- `<IRI>`: IRI identifying the project to be used
- `[VERSIONS_CONSTRAINT]`: A constraint on the allowable versions of a used project

## Options

- `--no-lock`: Do not automatically resolve usages (and generate lockfile)
- `--no-sync`: Do not automatically install dependencies
