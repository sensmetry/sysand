# `sysand env install`

Install project in `sysand_env`

## Usage

```sh
sysand env install [OPTIONS] <IRI> [VERSION]
```

## Description

Installs a given project and all it's dependencies in `sysand_env` for current project.

Current project is determined as in [sysand print-root](../root.md) and
if none is found uses the current directory instead.

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](../env.md).

## Arguments

- `<IRI>`: IRI identifying the project to be installed
- `[VERSION]`: Version to be installed

## Options

- `--path <PATH>`: Local path to interchange project
- `--allow-overwrite`: Allow overwriting existing installation
- `--allow-multiple`: Install even if another version is already installed
- `--no-deps`: Don't install any dependencies

{{#include ../partials/resolution_opts.md}}

{{#include ../partials/global_opts.md}}
