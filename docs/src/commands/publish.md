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
For `sysand publish`, only bearer token credentials
(`SYSAND_CRED_<X>_BEARER_TOKEN`) are used.
If no matching bearer token credentials are configured for the publish URL,
the command fails before making the upload request.

The package identifier used during publish is derived from project metadata.
Before publishing, ensure `version`, `publisher`, and `name` follow these rules:

- `version`: must be a valid Semantic Versioning 2.0 version.

- `publisher`: 3-50 characters, letters and numbers only, with optional single
  spaces or hyphens between words, and must start and end with a letter or
  number.
- `name`: 3-50 characters, letters and numbers only, with optional single
  spaces, hyphens, or dots between words, and must start and end with a letter
  or number.

`name` dots are preserved in the published identifier (they are not normalized
away).

## Arguments

- `[PATH]`: Path to the `.kpar` file to publish. If not provided, looks for
  a KPAR in the output directory matching the current project's name and
  version (e.g. `output/<name>-<version>.kpar`).

## Options

- `--index <URL>`: URL of the package index to publish to. Defaults to the
  configured default index URL, otherwise the first configured index URL, or
  `https://beta.sysand.org`.

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
