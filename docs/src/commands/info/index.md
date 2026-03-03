# `sysand info index`

Get project index

## Usage

```sh
sysand info index [OPTIONS]
```

## Description

Prints the list of indexed source files for the current project.

This field is read-only via `sysand info`. To add or remove files from the index,
use [`sysand include`](../include.md) and [`sysand exclude`](../exclude.md).

## Options

- `--numbered`: Print the list with item numbers

{{#include ../partials/global_opts.md}}
