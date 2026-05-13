# `sysand env`

Create a local `.sysand` environment for installing dependencies

## Usage

```sh
sysand env [OPTIONS]
```

## Description

Creates an empty `.sysand` environment for the current project if no existing
environment can be found, and otherwise leaves it unchanged.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

{{#include ./partials/global_opts.md}}
