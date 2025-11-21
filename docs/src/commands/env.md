# `sysand env`

Create a local `sysand_env` environment for installing dependencies

## Usage

```sh
sysand env [OPTIONS]
```

## Description

Creates an empty `sysand_env` environment for the current project if no existing
environment can be found, and otherwise leaves it unchanged.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

{{#include ./partials/global_opts.md}}
