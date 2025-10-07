# Commands

An overview of commands available for `sysand`.

## `sysand init`

Create new project in current directory

### Usage

```sh
sysand init [OPTIONS]
```

### Description

Create new project in current directory, i.e. create `.project.json` and
`.meta.json` files.

### Options

- `--name <NAME>`: Set the project name. Defaults to the directory name.
- `--version <VERSION>`: Set the version. Defaults to `0.0.1`.

## `sysand new`

Create new project in given directory

### Usage

```sh
sysand new [OPTIONS] <PATH>
```

### Description

Create new project at `<PATH>`, i.e. a new directory containing `.project.json`
and `.meta.json`.

### Arguments

- `<PATH>`: Path to the new project.

### Options

- `--name <NAME>`: Set the project name. Defaults to the directory name.
- `--version <VERSION>`: Set the version. Defaults to `0.0.1`.

## `sysand add`

Add usage to project information

### Usage

```sh
sysand add [OPTIONS] <IRI> [VERSIONS_CONSTRAINT]
```

### Description

Adds IRI and optional version constraint to list of usages in the project
information file `.project.json`. By default this will also update the lockfile
and sync the local environment (creating one if not already present).

### Arguments

- `<IRI>`: IRI identifying the project to be used
- `[VERSIONS_CONSTRAINT]`: A constraint on the allowable versions of a used project

### Options

- `--no-lock`: Do not automatically resolve usages (and generate lockfile)
- `--no-sync`: Do not automatically install dependencies

## `sysand remove`

Remove usage from project information

### Usage

```sh
sysand remove [OPTIONS] <IRI>
```

### Description

Removes all instances of IRI from list usages in the project information file
`.project.json`. By default this will also update the lockfile and sync the local
environment (creating one if not already present).

### Arguments

- `<IRI>`: IRI identifying the project usage to be removed

## `sysand include`

Include model interchange files in project metadata

### Usage

```sh
sysand include [OPTIONS] [PATHS]...
```

### Description

Takes all files given by PATHS and adds them to project metadata index and
checksum list in `.meta.json` for the current project. By default the checksum is
not computed and is left blank (with algorithm as `"None"`).

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Arguments

- `[PATHS]...`: File(s) to include in the project

### Options

- `--compute-checksum`: Compute and add file (current) SHA256 checksum
- `--no-index-symbol`: Do not detect and add top level symbols to index

## `sysand exclude`

Exclude model interchange files from project metadata

### Usage

```sh
sysand exclude [OPTIONS] [PATHS]...
```

### Description

Takes all files given by PATHS and removes all instances of them to project
metadata index and checksum list in `.meta.json` for the current project.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Arguments

- `[PATHS]...`: File(s) to exclude from the project

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

### Arguments

- `[PATH]`: Path giving where to put the finished KPAR. Defaults to
  `output/<project name>.kpar` or `output/project.kpar` if no name is found

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

### Arguments

- `<IRI>`: IRI identifying the project to be installed
- `[VERSION]`: Version to be installed

### Options

- `--path <PATH>`: Local path to interchange project
- `--allow-overwrite`: Allow overwriting existing installation
- `--allow-multiple`: Install even if another version is already installed
- `--no-deps`: Don't install any dependencies

## `sysand env uninstall`

Uninstall project in `sysand_env`

### Description

Uninstalls a given project in `sysand_env`.

### Usage

```sh
sysand env uninstall [OPTIONS] <IRI> [VERSION]
```

### Arguments

- `<IRI>`: IRI identifying the project to be uninstalled
- `[VERSION]`: Version to be uninstalled

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
sysand env sources [OPTIONS] <IRI> [VERSION]
```

### Description

Prints the paths to the source files (separated by newlines) for an installed
project and (optionally) its dependencies. Is intended to be machine readable.

By default sources for standard libraries are not included, as they are
typically shipped with your language implementation.

### Arguments

- `<IRI>`: IRI of the (already installed) project for which to enumerate source files
- `[VERSION]`: Version of project to list sources for

### Options

- `--no-deps`: Do not include sources for dependencies
- `--include-std`: Include (installed) KerML/SysML standard libraries

## `sysand sync`

Sync `sysand_env` to lockfile, creating a lockfile and `sysand_env if needed

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
sysand info [OPTIONS] <COMMAND>
```

### Description

Prints out the information contained in the .project.json file for the specified
project, defaulting to current project if no project is specified. Optionally an
extra command can be given to gte or set values in `.project.json` and `.meta.json`.

Current project is determined as in [sysand print-root](#sysand-print-root) and
if none is found uses the current directory instead.

### Options

- `--path <PATH>`: Use the project at the given path instead of the current project
- `--iri <PATH>`: Use the project with the given IRI/URI/URL instead of the
  current project
- `--auto-location <AUTO_LOCATION>`: Use the project with the given location, trying
  to parse it as an IRI/URI/URL and otherwise falling back to a local path
- `--no-normalise`: Do not try to normalise the IRI/URI when resolving

## `sysand sources`

List source files for the current project and (optionally) its dependencies

### Usage

```sh
sysand sources [OPTIONS]
```

### Description

Prints the paths to the source files (separated by newlines) for the current
project and (optionally) its dependencies. Is intended to be machine readable.

By default sources for standard libraries are not included, as they are
typically shipped with your language implementation.

Current project is determined as in [sysand print-root](#sysand-print-root)
and if none is found uses the current directory instead.

### Options

- `--no-deps`: Do not include sources for dependencies
- `--include-std`: Include (installed) KerML/SysML standard libraries

### Dependency options

- `--use-index [<USE_INDEX>...]`: Use an index when resolving this usage
- `--no-index`: Do not use any index when resolving this usage
- `--include`: Include usages of KerML/SysML standard libraries if present

## `sysand print-root`

Prints the root directory of the current project

### Usage

```sh
sysand print-root [OPTIONS] <IRI>
```

### Description

Tries to find the current project by starting in the current directory end then
iteratively going up the parent directories until a project directory is found.

A project directory is considered to be any directory containing either a
`.project.json` or a `.meta.json file`.
