# `sysand info maintainer`

Get or manipulate the list of maintainers of the project

## Usage

```sh
sysand info maintainer [OPTIONS]
```

## Description

Prints the list of maintainers of the given project. With modifying options, updates the list.

## Options

- `--numbered`: Prints a numbered list
- `--set <MAINTAINER>`: Replace the entire list with a single maintainer
- `--add <MAINTAINER>`: Append a maintainer to the list
- `--remove <N>`: Remove the maintainer at position N (1-based, as shown by `--numbered`)
- `--clear`: Remove all maintainers

{{#include ../partials/global_opts.md}}
