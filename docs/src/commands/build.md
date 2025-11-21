# `sysand build`

Build a KerML Project Archive (KPAR)

## Usage

```sh
sysand build [OPTIONS] [PATH]
```

## Description

Creates a KPAR file from the current project.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Arguments

- `[PATH]`: Path giving where to put the finished KPAR. Defaults to
  `output/<project name>.kpar` or `output/project.kpar` if no name is found

{{#include ./partials/global_opts.md}}
