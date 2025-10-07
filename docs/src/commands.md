# Commands

An overview of commands available for `sysand`.

## `sysand new`

Create new project in given directory

### Usage

```sh
sysand new [OPTIONS] <DIR>
```

### Description

Create new project at `<PATH>`, i.e. a new directory containing .project.json
and .meta.json.

## `sysand init`

Create new project in current directory

### Usage

```sh
sysand init [OPTIONS]
```

### Description

Create new project in current directory, i.e. create .project.json and
.meta.json files.

## `sysand add`

Add usage to project information

### Usage

```sh
sysand add [OPTIONS] <IRI> [VERSIONS_CONSTRAINT]
```

### Description

Adds IRI and optional VERSIONS_CONSTRAINT to list of usages in the project
information file .project.json. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

## `sysand remove`

Remove usage from project information

### Usage

```sh
sysand remove [OPTIONS] <IRI>
```

### Description

Removes all instances of IRI from list usages in the project information file
.project.json. By default this will also update the lockfile and sync the local
environment (creating one if not already present).

## `sysand include`

Include model interchange files in project metadata

### Usage

```sh
sysand include [OPTIONS] [PATHS]...
```

### Description

Takes all files given by PATHS and adds them to project metadata index and
checksum list in .meta.json for the current project. By default the checksum is
not computed and is left blank (with algorithm as "None").

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand exclude`

Exclude model interchange files from project metadata

### Usage

```sh
sysand exclude [OPTIONS] [PATHS]...
```

### Description

Takes all files given by PATHS and removes all instances of them to project
metadata index and checksum list in .meta.json for the current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand build`

Build a KerML Project Archive (KPAR)

### Usage

```sh
sysand build [OPTIONS] [PATH]
```

### Description

Creates a KPAR file from the current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand lock`

Create or update lockfile

### Usage

```sh
sysand lock [OPTIONS]
```

### Description

Resolves all usages in project information for current project and generates a
lockfile `SysandLock.toml` in the project root directory with exact versions and
sources for all dependencies.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand env`

Create a local `sysand_env` environment for installing dependencies

### Usage

```sh
sysand env [OPTIONS]
```

### Description

Creates an empty `sysand_env` environment for the current project if no existing
environment can be found, and otherwise leaves it unchanged.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand env install`

Install project in `sysand_env`

### Usage

```sh
sysand env install [OPTIONS] <IRI> [VERSION]
```

### Description

Installs a given project and all it's dependencies in `sysand_env` for current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](#sysand-env).

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

### Usage

```sh
sysand env list [OPTIONS]
```

### Description

List projects installed in `sysand_env` by IRI and version.

## `sysand env sources`

List source files for an installed project and (optionally) its dependencies

### Usage

```sh
sysand env sources [OPTIONS] <IRI>
```

### Description

Prints the paths to the source files (separated by newlines) for an installed
project and (optionally) its dependencies. Is intended to be machine readable.

## `sysand sync`

Sync env to lockfile, creating a lockfile if none is found

### Usage

```sh
sysand sync [OPTIONS]
```

### Description

Installs all projects in the current projects lockfile `SysandLock.toml` from
the sources listed, into the local `sysand_env` environment.

If a lockfile is not found, a new lockfile will be generated from the usages in
the project information in the same way as [sysand lock](#sysand-lock).

If no existing `sysand_env` is found, a new one will be created in the same way
as [sysand env](#sysand-env).

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand info`

Resolve and describe current project or one at at a specified path or IRI/URL

### Usage

```sh
sysand info [OPTIONS]
sysand info [OPTIONS] [COMMAND]
```

### Description

Prints out the information contained in the .project.json file for the specified
project, defaulting to current project if no project is specified. Optionally an
extra command can be given to gte or set values in .project.json and .meta.json.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

## `sysand sources`

List source files for the current project and (optionally) its dependencies

### Usage

```sh
sysand sources [OPTIONS] <IRI>
```

### Description

Prints the paths to the source files (separated by newlines) for the current
project and (optionally) its dependencies. Is intended to be machine readable.

Current project is determined as in [sysand print-root](#sysand-print-root)
and if none is found uses the current directory instead.

## `sysand print-root`

Prints the root directory of the current project

### Usage

```sh
sysand sources [OPTIONS] <IRI>
```

### Description

Tries to find the current project by starting in the current directory end then
iteratively going up the parent directories until a project directory is found.

A project directory is considered to be any directory containing either a
.project.json or a .meta.json file.
