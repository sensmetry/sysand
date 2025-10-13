# `sysand include`

Include model interchange files in project metadata

## Usage

```sh
sysand include [OPTIONS] [PATHS]...
```

## Description

Takes all files given by PATHS and adds them to project metadata index and
checksum list in `.meta.json` for the current project. By default the checksum is
not computed and is left blank (with algorithm as `"None"`).

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Arguments

- `[PATHS]...`: File(s) to include in the project

## Options

- `--compute-checksum`: Compute and add file (current) SHA256 checksum
- `--no-index-symbol`: Do not detect and add top level symbols to index
