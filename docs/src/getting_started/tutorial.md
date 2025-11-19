# Tutorial

## Model interchange projects

A model interchange project is a collection of SysML or KerML files with
additional metadata such as project name, versions, and the list of projects on
which it depends. To create a new project called `my_project` run:

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

## Source files

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

## Dependencies

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
    Usage: https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar
```

If we run `sysand source` again, it will now include all source files of the
set of (transitive) dependencies.

```console
$ sysand sources
/path/to/my_project/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Metaobjects.kerml
/path/to/my_project/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Performances.kerml
/path/to/my_project/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Links.kerml
/path/to/my_project/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/SpatialFrames.kerml
/path/to/my_project/sysand_env/7afe310696b522f251dc21ed6086ac4b50a663969fd1a49aa0aa2103d2a674ad/1.0.0.kpar/Clocks.kerml
...
```

## Environments

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

## Packaging projects for distribution

To package your project for distribution, run `sysand build`:

```console
$ sysand build
    Building kpar: /path/to/my_project/output/my_project.kpar
```

This command creates a `my_project.kpar` file that can be installed in a
different project using `sysand`.
