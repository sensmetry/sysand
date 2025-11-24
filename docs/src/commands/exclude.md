# `sysand exclude`

Exclude model interchange files from project metadata

## Usage

```sh
sysand exclude [OPTIONS] [PATHS]...
```

## Description

Takes all files given by PATHS and removes all instances of them to project
metadata index and checksum list in `.meta.json` for the current project.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Arguments

- `[PATHS]...`: File(s) to exclude from the project

{{#include ./partials/global_opts.md}}
