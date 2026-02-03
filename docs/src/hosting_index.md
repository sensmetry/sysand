# Self-hosting a project index

## Introduction

This section provides instructions on how to use Sysand CLI to host a
private Sysand project index. The guide will focus on running the index
on your local machine for testing purposes, but the general approach
applies to a more sophisticated production hosting as well. Get in touch
with your IT administrator to get this running on your company's infrastructure.

> [!note]
> This guide is only concerned about hosting the package files in the
> structure that Sysand CLI can understand. Hosting of a front-end website
> (such as [beta.sysand.org][sysand_index]) is not a part of this guide.

> [!warning]
> Sysand is in active development.
> The structure of indexes and `sysand_env` folders **can and will change**
> with future updates of Sysand. As long as Sysand is on version 0.x.y, we cannot
> guarantee backwards compatibility between different Sysand versions and
> indexes created using different Sysand versions.

The easiest way to host a project index from which to install packages is to
expose a `sysand_env` over HTTP, since indexes and `sysand_env` share the same
structure that is understood by Sysand.

## Create `sysand_env`

First, use the Sysand CLI to create a Sysand environment:
```sh
sysand env
```

This will create a `sysand_env/` folder in your current directory.

## Add packages to the environment

You can now install the packages you want to share into the Sysand environment.
For example, if you have a `MyProject.kpar` file in your current directory,
you can add it to the package index by:
```sh
sysand env install urn:kpar:my_project --path MyProject.kpar
```

This command will create an entry in the package index with the IRI of
`urn:kpar:my_project` that other people can then use to install your package.
With the `--path` argument, you point
to the `.kpar` file that you want to host on the package index.

> [!tip]
> Any IRI can be freely chosen here, just don't choose an IRI that could
> point to another resource, like the ones starting with `http(s)`, `file` or `ssh`.

By default, this command also installs all usages (dependencies) of `my_project`.
You can use the `--no-deps` argument to only install the package without
dependencies.

Repeat this step for as many times as you have packages (and their versions),
giving a unique IRI for each different package.

## Start an HTTP server

Once you install all the required packages, you can use Python and its
[built-in `http.server` module](https://docs.python.org/3/library/http.server.html)
to quickly start a simple HTTP server that will make the package index accessible
over the network. To do this, run:
```sh
python3 -m http.server -d sysand_env 8080
```

This command executes the `http.server` module on port `8080`, and tells the
module to expose the contents of the `sysand_env` folder to the network.

> [!important]
> Python's built-in `http.server` module is **not** intended for production use.

To set up a real production environment, ask your IT department for
guidelines or to do it for you.

## Try out your new index

You should now be able to access the package index through
[http://localhost:8080](http://localhost:8080).
To test it, create a new SysML v2 project in another directory by following
the [User Guide](tutorial.md).

Then, when adding a new usage to the project, use the `--index` argument
to point to your private package index instead of the public
[beta.sysand.org][sysand_index], for example:
```sh
sysand add urn:kpar:my_project --index http://localhost:8080
```

> [!important]
> `localhost` tells Sysand to look for the package index running on your
> machine. For connecting to other machines replace `localhost` by the
> address of the other machine, ensuring that networking and firewalls
> are correctly configured.

An alternative to `--index` argument is defining the custom index in
`sysand.toml`. See [Indexes](config/indexes.md) for details.

[sysand_index]: https://beta.sysand.org/
