# `sysand info checksum`

Get project source file checksums

## Usage

```sh
sysand info checksum [OPTIONS]
```

## Description

Prints the list of source file checksums for the given project.

This field is read-only via `sysand info`. Checksums are updated by
[`sysand include`](../include.md) and removed by [`sysand exclude`](../exclude.md).

## Options

- `--numbered`: Prints a numbered list

{{#include ../partials/global_opts.md}}
