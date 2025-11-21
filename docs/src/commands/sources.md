# `sysand sources`

List source files for the current project and (optionally) its dependencies

## Usage

```sh
sysand sources [OPTIONS]
```

## Description

Prints the paths to the source files (separated by newlines) for the current
project and (optionally) its dependencies. Is intended to be machine readable.

By default sources for standard libraries are not included, as they are
typically shipped with your language implementation.

Current project is determined as in [sysand print-root](root.md)
and if none is found uses the current directory instead.

{{#include ./partials/source_opts.md}}

{{#include ./partials/global_opts.md}}
