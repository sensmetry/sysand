# `sysand info usage`

Print project usages

## Usage

```sh
sysand info usage [OPTIONS]
```

## Description

Prints the list of usages (dependencies) of the given project.

This field is read-only via `sysand info`. To add or remove usages, use
[`sysand add`](../add.md) and [`sysand remove`](../remove.md).

## Options

- `--numbered`: Prints a numbered list

{{#include ../partials/global_opts.md}}
