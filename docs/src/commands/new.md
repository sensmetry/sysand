# `sysand new`

Create new project in given directory

## Usage

```sh
sysand new [OPTIONS] <PATH>
```

## Description

Create new project at `<PATH>`, i.e. a new directory containing `.project.json`
and `.meta.json`.

## Arguments

- `<PATH>`: Path to the new project.

## Options

- `--name <NAME>`: Set the project name. Defaults to the directory name.
- `--version <VERSION>`: Set the version. Defaults to `0.0.1`.
