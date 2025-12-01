# Sysand: a project and package manager for SysML v2 and KerML

> [!important]
> This is an early preview release, intended for early adopters
> to test, integrate, and give feedback. While we hope to keep the tool in a
> usable state, interfaces are subject to change and usability will likely not
> yet be representative of a stable release.

This repository contains Sysand, a [package
manager](https://en.wikipedia.org/wiki/Package_manager) for SysML v2 and KerML
similar to package managers for programming languages such as Pip for Python,
NPM for JavaScript, Maven for Java, and NuGet for .NET. Sysand is based on a
concept of a model interchange project, a slight generalization of a project
interchange file (`*.kpar`), defined in [KerML clause
10.3](https://www.omg.org/spec/KerML/1.0/Beta4/PDF#page=428).

Sysand can be used as a standalone tool through its command line interface (CLI)
or be integrated into other tools through one of its APIs (currently, Python and
Java are supported).

The following section provides basic information on how to use Sysand via CLI.
The later sections provide information relevant for potential contributors.

## Basic use via the command line interface

### Installation

Sysand is written in Rust programming language. To build it, [install
Rust](https://www.rust-lang.org/tools/install) and run the following command in
the terminal:

```sh
cargo install sysand --git=https://github.com/sensmetry/sysand.git
```

With Sysand installed, you can now create a model interchange project as shown
in the following subsection.

### Model interchange projects

A model interchange project is a collection of SysML v2 (`.sysml`) or KerML (`.kerml`)
files with additional metadata such as project name, versions, and the list of
projects on which it depends. To create a new project called `my_project` run:

```console
$ sysand new my_project
    Creating interchange project `my_project`
```

This creates a new directory (`my_project`) and populates it with a minimal
interchange project, consisting of two files `.project.json` and `.meta.json`.

Inside the directory, we can ask for a basic overview of the project.

```console
$ cd my_project
$ sysand info
Name: my_project
Version: 0.0.1
No usages.
```

### Source files

The project we created in the previous subsection contains no source files as
can be seen by running the following command:

```console
$ sysand sources
<NO OUTPUT>
```

Before we can add source files to the project, we need to create them. Create
`MyProject.sysml` file with the following content:

```sysml
package MyProject;
```

Now, we can add `MyProject.sysml` to our project by running the following
command:

```console
$ sysand include MyProject.sysml
   Including files: ["MyProject.sysml"]
```

The file will now be listed by `sysand sources`, which can serve as the input
to a SysML v2 processing environment.

```console
$ sysand sources
/path/to/my_project/MyProject.sysml
```

The following subsection shows how to add dependencies to our project.

### Dependencies

Effectively all projects depend on elements defined in other projects. The key
benefit of Sysand is that it can automatically manage project dependencies for
you.

KerML (and by extension in SysML v2) specification calls a project dependency a
usage. Each usage is identified by an [Internationalized Resource Identifier
(IRI)](https://en.wikipedia.org/wiki/Internationalized_Resource_Identifier) with
an optional version constraint. To add dependencies, use the `sysand add`
command. The simplest way to use it is to give an IRI to a package you want to
install from the [Sysand Package Index](https://sysand.org). You can find the
IRI (and the full install command) in the card of the package on the index
website. For example, to install the standard Function Library, run:

```console
$ sysand add urn:kpar:function-library
      Adding usage: urn:kpar:function-library
    Creating env
     Syncing env
  Installing urn:kpar:semantic-library 1.0.0
  Installing urn:kpar:data-type-library 1.0.0
  Installing urn:kpar:function-library 1.0.0
```

It is also possible to install packages from the URL that points to the `.kpar`
file as shown in the following snippet:

```console
$ sysand add https://www.omg.org/spec/KerML/20250201/Function-Library.kpar
      Adding usage: https://www.omg.org/spec/KerML/20250201/Function-Library.kpar
    Creating env
     Syncing env
  Installing https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar 1.0.0
  Installing https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar 1.0.0
  Installing https://www.omg.org/spec/KerML/20250201/Function-Library.kpar 1.0.0
```

Adding a dependency may take a few seconds to run, as it will find and install
the project (and any transitive usages) into a new local environment. Once
finished, this will have created a file called `sysand-lock.toml` and a directory
`sysand_env`. The former records the precise versions installed, so that the
same installation can be reproduced later. The latter directory will contain a
local installation of the added project, as well as any of its (transitive)
usages. `sysand-lock.toml` is sufficient to reproduce `sysand_env`; therefore, we
recommend checking in `sysand-lock.toml` into your version control system and
adding `sysand_env` to `.gitignore`.

We can confirm that the usage was successfully added by running the `info`
command again:

```console
$ sysand info
Name: my_project
Version: 0.0.1
Usages:
    https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar
```

If we run `sysand source` again, it will now include all source files of the
set of (transitive) dependencies.

```console
$ sysand sources
/Users/vakaras2/projects/tmp/sysand/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Metaobjects.kerml
/Users/vakaras2/projects/tmp/sysand/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Performances.kerml
/Users/vakaras2/projects/tmp/sysand/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Links.kerml
/Users/vakaras2/projects/tmp/sysand/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/SpatialFrames.kerml
/Users/vakaras2/projects/tmp/sysand/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Clocks.kerml
...
```

### Environments

When we executed `sysand add` in the previous subsection, it implicitly created
and synchronized an *environment* for us. For users familiar with Python, Sysand
environments serve the same purpose as Python virtual environments: they store
dependencies needed for a specific project.

We can see everything installed in the local environment using `sysand env
list`:

```console
$ sysand env list
https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar 1.0.0
https://www.omg.org/spec/KerML/20250201/Function-Library.kpar 1.0.0
https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar 1.0.0
```

If you want to recreate the environment on a new machine, make sure you have not
only your project files, but also `sysand-lock.toml` and execute the following
command:

```console
$ sysand sync
    Creating env
     Syncing env
  Installing https://www.omg.org/spec/KerML/20250201/Data-Type-Library.kpar 1.0.0
  Installing https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar 1.0.0
  Installing https://www.omg.org/spec/KerML/20250201/Function-Library.kpar 1.0.0
```

### Packaging projects for distribution

To package your project for distribution, run `sysand build`:

```console
$ sysand build
    Building kpar: /path/to/my_project/output/my_project.kpar
```

This command creates a `my_project.kpar` file that can be installed in a
different project using `sysand`.

## Hosting a project index

> [!important]
> The structure of indexes and `sysand_env` environments is still expected to
> change, and may currently not be compatible between sysand releases.

The easiest way to host a project index from which to install packages is to
expose a `sysand_env` over HTTP.

If you have an existing `sysand_env`, and you have a working Python 3 environment
you can test this with

```console
$ python3 -m http.server -d sysand_env 8080
Serving HTTP on 0.0.0.0 port 8080 (http://0.0.0.0:8080/) ...
```

> [!important]
> Python's built-in `http.server` module is *not* intended for production use.

Any project in the above `sysand_env` can now be used in `sysand add`, `sysand sync`,
`sysand env install`, etc., as long as the flag `--use-index http://localhost:8080`
is added (or soon by specifying it in `sysand.toml`!).

For example, to create an index to publish the above `my_project` project we can
create a fresh `sysand_env`.

```console
$ mkdir my_index
$ cd my_index
$ sysand env 
    Creating env
```

Now we install `my_project`, specifying the IRI/URL that you want to use to refer
to it:

```console
$ sysand env install urn:kpar:my_project --path /path/to/my_project/
  Installing urn:kpar:my_project 0.0.1
     Syncing env
```

By default, this will also install any usages (dependencies) of `my_project`, you
can use `--no-deps` to install only the project itself.

## Documentation

The "Sysand User Guide" is currently a work in progress. To preview make sure
you have `mdbook` installed (`cargo install mdbook`), then either run

```sh
mdbook build docs/
```

and open `docs/book/index.html`, or run

```sh
mdbook serve docs/
```

and open [localhost:3000](http://localhost:3000/).

## Contributing

### Development

Development instructions are provided in [DEVELOPMENT.md](DEVELOPMENT.md).

### Legal

For contributors' guidelines regarding legal matters, please see the
[CONTRIBUTING.md](CONTRIBUTING.md) file.

## Licensing

The implementation is dual-licensed under the MIT and Apache-2.0 licenses,
meaning users may choose to use the code under *either* license. Contributors
agree to provide contributed code under **both** licenses.

Sysand is maintained by [Sensmetry](https://www.sensmetry.com), with
contributions from the community. To see the complete list of contributors,
please see the git history.
