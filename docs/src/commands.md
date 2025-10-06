# Commands

An overview of commands available for `sysand`.

## `sysand new`

Create new project in given directory

### Description

Create new project at `<PATH>`, i.e. a new directory containing .project.json
and .meta.json.

### Usage

```sh
sysand new [OPTIONS] <DIR>
```

## `sysand init`

Create new project in current directory

### Description

Create new project in current directory, i.e. create .project.json and
.meta.json files.

### Usage

```sh
sysand init [OPTIONS]
```

## `sysand add`

Add usage to project information

### Description

Adds IRI and optional VERSIONS_CONSTRAINT to list of usages in the project
information file .project.json. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

### Usage

```sh
sysand add [OPTIONS] <IRI> [VERSIONS_CONSTRAINT]
```

## `sysand remove`

Remove usage from project information

### Description

Removes all instances of IRI from list usages in the project information file
.project.json. By default this will also update the lockfile and sync the local
environment (creating one if not already present).

### Usage

```sh
sysand remove [OPTIONS] <IRI>
```

## `sysand include`

Include model interchange files in project metadata

### Description

Takes all files given by PATHS and adds them to project metadata index and
checksum list in .meta.json for the current project. By default the checksum is
not computed and is left blank (with algorithm as "None").

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand include [OPTIONS] [PATHS]...
```

## `sysand exclude`

Exclude model interchange files from project metadata

### Description

Takes all files given by PATHS and removes all instances of them to project
metadata index and checksum list in .meta.json for the current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand exclude [OPTIONS] [PATHS]...
```

## `sysand build`

Build a KerML Project Archive (KPAR)

### Description

Creates a KPAR file from the current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand build [OPTIONS] [PATH]
```

## `sysand lock`

Create or update lockfile

### Description

Resolves all usages in project information for current project and generates a
lockfile `SysandLock.toml` in the project root directory with exact versions and
sources for all dependencies.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand lock [OPTIONS]
```

## `sysand env`

Create a local `sysand_env` environment for installing dependencies

### Description

Creates an empty `sysand_env` environment for the current project if no existing
environment can be found, and otherwise leaves it unchanged.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand env [OPTIONS]
```

## `sysand env install`

Install project in `sysand_env`

### Description

Installs a given project and all it's dependencies in `sysand_env` for current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](#sysand-env).

### Usage

```sh
sysand env install [OPTIONS] <IRI> [VERSION]
```

## `sysand env uninstall`

Uninstall project in `sysand_env`

### Description

Uninstalls a given project in `sysand_env`.

### Usage

```sh
sysand env uninstall [OPTIONS] <IRI> [VERSION]
```

## `sysand env list`

List projects installed in `sysand_env`

### Description

List projects installed in `sysand_env` by IRI and version.

### Usage

```sh
sysand env list [OPTIONS]
```

## `sysand env sources`

List source files for an installed project and (optionally) its dependencies

### Description

Prints the paths to the source files (separated by newlines) for an installed
project and (optionally) its dependencies. Is intended to be machine readable.

### Usage

```sh
sysand env sources [OPTIONS] <IRI>
```

## `sysand sync`

Sync env to lockfile, creating a lockfile if none is found

### Description

Installs all projects in the current projects lockfile `SysandLock.toml` from
the sources listed, into the local `sysand_env` environment.

If a lockfile is not found, a new lockfile will be generated from the usages in
the project information in the same way as [sysand lock](#sysand-lock).

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](#sysand-env).

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand sync [OPTIONS]
```

## `sysand info`

Resolve and describe current project or one at at a specified path or IRI/URL

### Description

Prints out the information contained in the .project.json file for the specified
project, defaulting to current project if no project is specified. Optionally an
extra command can be given to gte or set values in .project.json and .meta.json.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Usage

```sh
sysand info [OPTIONS]
sysand info [OPTIONS] [COMMAND]
```

## `sysand sources`

List source files for the current project and (optionally) its dependencies

### Description

Prints the paths to the source files (separated by newlines) for the current
project and (optionally) its dependencies. Is intended to be machine readable.

Current project is determined as in [sysand print-root](#sysand-print-root)
and if none is found uses the current directory instead.

### Usage

```sh
sysand sources [OPTIONS] <IRI>
```

## `sysand print-root`

Prints the root directory of the current project

### Description

Tries to find the current project by starting in the current directory end then
iteratively going up the parent directories until a project directory is found.

A project directory is considered to be any directory containing either a
.project.json or a .meta.json file.

### Usage

```sh
sysand sources [OPTIONS] <IRI>
```
