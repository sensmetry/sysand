# `sysand lock`

Create or update lockfile

## Usage

```sh
sysand lock [OPTIONS]
```

## Description

Resolves all usages in project information for current project and generates a
lockfile `sysand-lock.toml` in the project root directory with exact versions and
sources for all dependencies.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

{{#include ./partials/resolution_opts.md}}

{{#include ./partials/global_opts.md}}
