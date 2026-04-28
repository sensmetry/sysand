# `sysand publish`

Publish a KPAR to a sysand package index.

## Usage

```sh
sysand publish --index <URL> [PATH]
```

## Description

Publishes a `.kpar` file to a sysand-compatible package index. The project
must be built first using [`sysand build`](build.md).

Authentication is required. See [Authentication](../authentication.md) for
how to configure credentials.
For `sysand publish`, only bearer token credentials
(`SYSAND_CRED_<X>_BEARER_TOKEN`) are used.
If no matching bearer token credentials are configured for the publish URL,
the command fails before making the upload request.

`--index` is required for `sysand publish`.

The package identifier used during publish is derived from project metadata.
Before publishing, ensure `publisher`, `name`, `version`, and `license` follow
these rules:

- `publisher`: 3-50 characters, ASCII letters and numbers only, with optional
  single spaces or hyphens between words, and must start and end with a letter
  or number.
- `name`: 3-50 characters, ASCII letters and numbers only, with optional single
  spaces, hyphens, or dots between words, and must start and end with a letter
  or number.
- `version`: must be a valid Semantic Versioning 2.0 version.
- `license`: required and must be a valid
  [SPDX license expression](https://spdx.github.io/spdx-spec/latest/annexes/spdx-license-expressions/).
  See [Project metadata: `license`](../metadata.md#license) for examples.

`name` dots are preserved in the published identifier (they are not normalized
away).

## Arguments

- `[PATH]`: Path to the `.kpar` file to publish. If not provided, looks for
  a KPAR in the output directory matching the current project's name and
  version (e.g. `output/<name>-<version>.kpar`).

## Options

- `--index <URL>`: URL of the package index to publish to. Required.
  This is either a directory that contains `sysand-index-config.json`, or the
  index root that contains `index.json` (for example, `https://sysand.org` or
  `https://my-index.example.com/index`).

{{#include ./partials/global_opts.md}}

## Examples

Build and publish the current project:

```sh
sysand build
sysand publish --index https://sysand.org
```

Publish a specific KPAR file:

```sh
sysand publish --index https://sysand.org ./my-project-1.0.0.kpar
```

Publish to a custom index:

```sh
sysand publish --index https://my-index.example.com
```

## See Also

- [`sysand build`](build.md) — Build a KPAR from a project
- [Authentication](../authentication.md) — Configure credentials
- [Publishing a package](../publishing.md) — Publishing guide
