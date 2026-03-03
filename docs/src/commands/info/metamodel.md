# `sysand info metamodel`

Get or set the metamodel of the project

## Usage

```sh
sysand info metamodel [OPTIONS]
```

## Description

Prints the metamodel of the current project. With modifying options, updates the metamodel.

There are two ways to set the metamodel:

**Standard SysML v2 or KerML metamodel** — use `--set` with an optional `--release`:

```sh
sysand info metamodel --set sysml
sysand info metamodel --set kerml --release 20250201
```

**Custom metamodel URI** — use `--set-custom` (mutually exclusive with `--set`):

```sh
sysand info metamodel --set-custom https://example.com/my-metamodel/1.0
```

## Options

- `--set <KIND>`: Set a standard metamodel (`sysml` or `kerml`)
- `--release <YYYYMMDD>`: Official release of the metamodel (requires `--set`; default: `20250201`); conflicts with `--release-custom`
- `--release-custom <YYYYMMDD>`: Custom release date (requires `--set`; conflicts with `--release`)
- `--set-custom <METAMODEL>`: Set an arbitrary metamodel URI (conflicts with `--set`, `--release`, `--release-custom`)
- `--clear`: Remove the metamodel field (conflicts with `--set`)

{{#include ../partials/global_opts.md}}
