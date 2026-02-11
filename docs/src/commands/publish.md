# `sysand publish`

Publish a KPAR to the sysand package index.

## Usage

```sh
sysand publish [OPTIONS] [PATH]
```

## Description

Publishes a `.kpar` file to a sysand-compatible package index. The project
must be built first using [`sysand build`](build.md).

Authentication is required. See [Authentication](../authentication.md) for
how to configure credentials.

## Arguments

- `[PATH]`: Path to the `.kpar` file to publish. If not provided, looks for
  a KPAR in the output directory matching the current project's name and
  version (e.g. `output/<name>-<version>.kpar`).

## Options

- `--index <URL>`: URL of the package index to publish to. Defaults to the
  first index URL from configuration or `https://beta.sysand.org`.

{{#include ./partials/global_opts.md}}

## Examples

Build and publish the current project:

```sh
sysand build
sysand publish
```

Publish a specific KPAR file:

```sh
sysand publish ./my-project-1.0.0.kpar
```

Publish to a custom index:

```sh
sysand publish --index https://my-index.example.com
```

## See Also

- [`sysand build`](build.md) — Build a KPAR from a project
- [Authentication](../authentication.md) — Configure credentials
- [Publishing a package](../publishing.md) — Publishing guide
