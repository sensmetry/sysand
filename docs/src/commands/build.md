# `sysand build`

Build a KerML Project Archive (KPAR). If executed in a workspace outside of a
project, builds all projects in the workspace.

## Usage

```sh
sysand build [OPTIONS] [PATH]
```

## Description

Creates a KPAR file from the current project.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Arguments

- `[PATH]`: Path for the finished KPAR or KPARs. When building a
  workspace, it is a path to the folder to write the KPARs to
  (default: `<current-workspace>/output`). When building a single
  project, it is a path to the KPAR file to write (default
  `<current-workspace>/output/<project name>-<version>.kpar` or
  `<current-project>/output/<project name>-<version>.kpar` depending
  on whether the current project belongs to a workspace or not).

## Options

-  `-a`, `--allow-path-usage`  Allow usages of local paths (`file://`).
  Warning: using this makes the project not portable between different
  computers, as `file://` URL always contains an absolute path.
  For multiple related projects, consider using a workspace instead

{{#include ./partials/global_opts.md}}
