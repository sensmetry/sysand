# `sysand info index`

Get project index

## Usage

```sh
sysand info index [OPTIONS]
```

## Description

Prints the list of indexed source files for the given project.

This field is read-only via `sysand info`. To add or remove files from the index,
use [`sysand include`](../include.md) and [`sysand exclude`](../exclude.md).

## Options

- `--numbered`: Prints a numbered list

{{#include ../partials/global_opts.md}}
