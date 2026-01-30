# Tutorial

This section will show a basic way of using Sysand CLI to manage SysML v2 projects.
More detailed guides are coming soon.


## Initialize a new project using Sysand CLI

A model interchange project is a collection of SysML v2 (`.sysml`) or KerML (`.kerml`)
files with additional metadata such as project name, versions, and the list of
projects on which it depends. To create a new project called `my_project` run

```console
$ sysand init my_project
    Creating interchange project `my_project`
```

This command will create a new directory (`my_project`) and populate it with
a minimal interchange project, consisting of two files: `.project.json` and
`.meta.json`. To instead create a new project in the current directory, omit
the path (`my_project` in this case). In that case, the project's name will
be the same as the current directory name.


## Inspect the project

Sysand can show some information about the project that was just created
with the following commands:

```console
$ cd my_project
$ sysand info
Name: my_project
Version: 0.0.1
No usages.
```

```console
$ sysand sources
*NO OUTPUT*
```

Currently, the project has no usages (dependencies) and no source files of its own.

## Add source files

For a project to be useful, it needs to have some `.sysml`/`.kerml` files of its
own. Create a `MyProject.sysml` file with the following contents:

```text
package MyProject;
```

Now, you can add `MyProject.sysml` to the project by running the following command:

```console
$ sysand include MyProject.sysml
   Including file: `MyProject.sysml`
```

The source file is now a part of the project, which `sysand sources` can confirm:

```console
$ sysand sources
/path/to/my_project/MyProject.sysml
```

## Add usages (dependencies)

KerML (and by extension SysML v2) specification calls a project dependency a
"usage". All projects will have at least one dependency on the SysML v2 standard library itself (TODO: link to standard packages section in index), and many will have
dependencies on more SysML v2 projects. The key benefit of Sysand is that it can
automatically manage project dependencies for you.

Each usage is identified by an [Internationalized Resource Identifier][iri]
(IRI) with an optional version constraint. To add dependencies, use the `sysand
add` command. The simplest way to use it is to give an IRI to a package you want
to install from the [Sysand Package Index][index]. You can find the IRI (and the full
install command) in the card of the package on the index. It is also possible
to install packages from the URL that points to the `.kpar` file or to a directory
that contains the project.

[iri]: https://en.wikipedia.org/wiki/Internationalized_Resource_Identifier
[index]: https://beta.sysand.org/

Install usage `SYSMOD` from Sysand Package Index:

```sh
sysand add urn:kpar:sysmod
```

Install usage from URL (TODO: where to link? only std lib is a reputable source of kpars;
linking to direct links in the index would look bad):

```sh
sysand add https://www.omg.org/spec/KerML/20250201/Function-Library.kpar
```

This may take a few seconds to run, as Sysand needs to download the
linked project (and its usages as well) into a new local environment.
Once finished, a file `sysand-lock.toml` and a directory `sysand_env`
will be created. The former records the precise versions of the external
projects installed, so that the same installation can be reproduced later.
The latter directory stores the added project and its usages.

```console
$ sysand info
Name: my_project
Version: 0.0.1
Usages:
    urn:kpar:sysmod
    TODO: add second usage from URL here
```

Running `sysand sources` again will list all the `.sysml files` from both the
current project and its (transitive) dependencies.

```console
$ sysand sources
warning: SysML v2/KerML standard library packages are omitted by default.
         If you want to include them, pass `--include-std` flag
/path/to/my_project/MyProject.sysml
/path/to/my_project/sysand_env/a0aacee34dd4cd5e2d07ab43d5e30772ec86dbf3c8fafb033bad338dd7a0f02e/5.0.0-alpha.2.kpar/SYSMOD.sysml
```

> [!note]
> SysML v2 and KerML standard libraries are usually provided by the tooling
> used to develop the models. For this reason Sysand will not install or show
> standard libraries (or their files) in its output, unless specifically
> requested with `--include-std`

> [!tip]
> `sysand-lock.toml` is sufficient to reproduce `sysand_env` on any computer;
> therefore, we recommend checking in `sysand-lock.toml` into your version
> control system and adding `sysand_env` to `.gitignore` (or equivalent).

## List installed dependencies

When we executed `sysand add` in the previous subsection, it implicitly created
and synchronized an environment for us. For users familiar with Python, Sysand
environments serve the same purpose as Python virtual environments: they store
dependencies needed for a specific project.

We can see everything installed in the local environment using `sysand env list`:

```console
$ sysand env list
`urn:kpar:sysmod` 5.0.0-alpha.2
```

> [!note]
> Environment may contain packages that are not (transitive) dependencies of
> the current project, because projects can also be installed into the
> environment without adding them to project usages. Also, projects are never
> automatically removed from the environment, so it may contain projects that
> are no longer used by the current project.

If you want to recreate (required part of) the environment on a new machine,
make sure you have not only your project files, but also `sysand-lock.toml`
and execute the following command:

```console
$ sysand sync
    Creating env
warning: Direct or transitive usages of SysML v2/KerML standard library packages are
         ignored by default. If you want to process them, pass `--include-std` flag
     Syncing env
  Installing `urn:kpar:sysmod` 5.0.0-alpha.2
```

## Package the project

After the project reaches some maturity level, there might be a need to
package it to a `.kpar` file for sharing with others (either through
the Sysand Package Index or otherwise). The `.kpar` file can be built by running:

```console
$ sysand build
    Building kpar `/path/to/my_project/output/my_project-0.0.1.kpar`
$ ls output/
my_project-0.0.1.kpar
```

Sysand CLI creates a new directory `output` and puts the generated `.kpar` file
there. The created `.kpar` file can then be installed or added as a usage for
another project:

```console
$ sysand init my_other_project
...
$ cd my_other_project/
$ sysand add file:///path/to/my_project/output/my_project-0.0.1.kpar
...
```
