# `sysand print-root`

Prints the root directory of the current project

## Usage

```sh
sysand print-root [OPTIONS] <IRI>
```

## Description

Tries to find the current project by starting in the current directory end then
iteratively going up the parent directories until a project directory is found.

A project directory is considered to be any directory containing either a
`.project.json` or a `.meta.json file`.
