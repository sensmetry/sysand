# `sysand index add`

Add a KPAR to a local sysand index

## Usage

```sh
sysand index add [OPTIONS] --kpar-path <KPAR_PATH> [IRI]
```

## Arguments

- `[IRI]`: Project identifier. Default is `pkg:sysand/<publisher>/<name>`, if publisher is
  specified in .project.json. Omitting both publisher and IRI is an error

## Options

- `--kpar-path <KPAR_PATH>`: Path to KPAR
- `--index-root <INDEX_ROOT>`: Path to the index directory. If not provided, current working directory is used

{{#include ../partials/global_opts.md}}
