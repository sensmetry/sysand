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

When adding a usage with a path or URL the configuration file will be
automatically updated with the appropriate project source override as described
in [Dependencies](../config/dependencies.md). The configuration file updated
will either be the one given with `--config-file` or (if `--no-config` is not
present) the `sysand.toml` at the root of the project. If no configuration file
is given and `--no-config` is set the usage will be added to the project but no
source will be configured so future syncing will not take this into account.

## Arguments

- `<IRI>`: IRI/URI/URL identifying the project to be used. See
  [`usage` field](../metadata.md#usage) for details.
- `[VERSION_CONSTRAINT]`: A constraint on the allowable versions of a used
  project. Assumes that the project uses Semantic Versioning. See
  [`versionConstraint`](../metadata.md#versionconstraint) for details

## Options

- `--no-lock`: Do not automatically resolve usages (and generate lockfile)
- `--no-sync`: Do not automatically install dependencies
- `--local-src <LOCAL_SRC>`: Path to local interchange project
- `--local-kpar <LOCAL_KPAR>`: Path to local interchange project archive (KPAR)
- `--remote-src <REMOTE_SRC>`: URL to remote interchange project
- `--remote-kpar <REMOTE_KPAR>`: URL to remote interchange project archive (KPAR)

{{#include ./partials/resolution_opts.md}}

{{#include ./partials/global_opts.md}}
