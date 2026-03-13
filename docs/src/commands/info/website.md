# `sysand info website`

Get or set the website of the project

## Usage

```sh
sysand info website [OPTIONS]
```

## Description

Prints the website of the given project. With `--set` or `--clear`, modifies the website.

The value must be a valid IRI/URI/URL.

## Options

- `--set <URI>`: Set the website. Must be a valid IRI/URI/URL
- `--clear`: Remove the project website

{{#include ../partials/global_opts.md}}
