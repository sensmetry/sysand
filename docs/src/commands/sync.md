# `sysand sync`

Sync `sysand_env` to lockfile, creating a lockfile and `sysand_env if needed

## Usage

```sh
sysand sync [OPTIONS]
```

## Description

Installs all projects in the current projects lockfile `SysandLock.toml` from
the sources listed, into the local `sysand_env` environment.

If a lockfile is not found, a new lockfile will be generated from the usages in
the project information in the same way as [sysand lock](lock.md).

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](env.md).

Current project is determined as in [sysand print-root](root.md) and
if none is found uses the current directory instead.
