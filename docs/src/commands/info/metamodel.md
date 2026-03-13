# `sysand info metamodel`

Get or set the metamodel of the project

## Usage

```sh
sysand info metamodel [OPTIONS]
```

## Description

Prints the metamodel of the given project. With modifying options, updates the metamodel.

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

- `--set <KIND>`: Set a SysML v2 or KerML metamodel. To set a custom metamodel, use `--set-custom`
- `--release <YYYYMMDD>`: Choose the release of the SysML v2 or KerML metamodel. SysML 2.0 and KerML 1.0 have the same release dates (default: `20250201`)
- `--release-custom <YYYYMMDD>`: Choose a custom release of the SysML v2 or KerML metamodel
- `--set-custom <METAMODEL>`: Set a custom metamodel. To set a SysML v2 or KerML metamodel, use `--set`
- `--clear`: Remove the metamodel field

{{#include ../partials/global_opts.md}}
