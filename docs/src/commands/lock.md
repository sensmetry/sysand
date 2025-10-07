# `sysand lock`

Create or update lockfile

## Usage

```sh
sysand lock [OPTIONS]
```

## Description

Resolves all usages in project information for current project and generates a
lockfile `SysandLock.toml` in the project root directory with exact versions and
sources for all dependencies.

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.

## Dependency options

- `--use-index [<USE_INDEX>...]`: Use an index when resolving this usage
- `--no-index`: Do not use any index when resolving this usage
- `--include`: Include usages of KerML/SysML standard libraries if present
