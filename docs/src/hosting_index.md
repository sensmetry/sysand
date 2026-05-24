# Self-hosting a Project Index

## Introduction

This section provides instructions on how to use Sysand CLI to host a private
Sysand project index. The guide describes three ways to run the index:

- [Local Machine](#local-machine) -- running a HTTP file server on your machine
- [GitHub](#github) -- using a GitHub repository
- [GitLab](#gitlab) -- using GitLab Pages

> [!note]
> This guide is only concerned about hosting the project files in the
> structure that Sysand CLI can understand. Hosting of a front-end website
> (such as [beta.sysand.org][sysand_index]) is not a part of this guide.

> [!warning]
> Sysand is in active development. The structure of indexes and
> `.sysand` folders **can and will change** with future updates of Sysand. As
> long as Sysand is on version 0.x.y, we cannot guarantee backwards
> compatibility between different Sysand versions and indexes created using
> different Sysand versions.

## Local Machine

This part of the guide focuses on running a Sysand index on your local machine
for testing purposes, but the general approach applies to a more sophisticated
production hosting as well. Get in touch with your IT administrator to get this
running on your company's infrastructure.

The easiest way to host a project index from which to install projects is to
create a local sysand index and expose it over HTTP.

### Create Local Index

First, use the Sysand CLI to create a Sysand environment:

```sh
sysand index init
```

This will initialize a sysand index in your current directory.

### Add Project to the Index

You can now add the projects you want to share into the Sysand index.
For example, if you have a `MyProject.kpar` file in your current directory,
you can add it to the project index by (provided `.project.json` specifies
`publisher` field):

```sh
sysand index add --kpar-path MyProject.kpar
```

This command will create an entry in the project index with the IRI of
`pkg:sysand/my-publisher/my-project-name` that other people can then use to install
your project. The `publisher` and `name` values are normalized values from fields
in `.project.json`.

> [!tip]
> If you don't specify `publisher` field in `.project.json`, you must provide
> IRI as a positional argument, for example
> `sysand index add my:iri/my-project --kpar-path MyProject.kpar`.
> `publisher` is person or organization that publishes the project. It is
> currently not in the KerML specification, we will propose adding it as a
> mandatory field.

> [!tip]
> Any IRI can be freely chosen for the `--iri` argument, just don't choose an IRI
> that could point to another resource, like the ones starting with `http(s)`,
> `file` or `ssh`. Also `pkg:sysand/<publisher>/<name>` IRI can only be chosen
> for projects which specify `publisher` in `.project.json`.

Repeat this step for as many times as you have projects (and their versions),
giving a unique IRI for each different project.

### Yank or Remove

You can also yank a project version, remove a project version, or remove the
entire project. See [yank command](./commands/index/yank.md) and
[remove command](./commands/index/remove.md) for more details.

### Start an HTTP server

Once you install all the required projects, you can use Python and its
[built-in `http.server` module](https://docs.python.org/3/library/http.server.html)
to quickly start a simple HTTP server that will make the project index accessible
over the network. To do this, run:

```sh
python3 -m http.server 8080
```

This command executes the `http.server` module on port `8080`, and tells the
module to expose the contents of the current folder to the network.

> [!important]
> Python's built-in `http.server` module is **not** intended for production use.

To set up a real production environment, ask your IT department for
guidelines or to do it for you.

### Sysand Client Setup

You should now be able to access the project index through
[http://localhost:8080](http://localhost:8080).
To test it, create a new SysML v2 project in another directory by following
the [User Guide](tutorial.md).

Then, when adding a new usage to the project, use the `--index` argument
to point to your private project index instead of the public
[beta.sysand.org][sysand_index], for example:

```sh
sysand add pkg:sysand/my-publisher/my-project --index http://localhost:8080
```

> [!important]
> `localhost` tells Sysand to look for the project index running on your
> machine. For connecting to other machines replace `localhost` by the
> address of the other machine, ensuring that networking and firewalls
> are correctly configured.

An alternative to `--index` argument is defining the custom index in
`sysand.toml`. See [Indexes](config/indexes.md) for details.

## GitHub

This part of the guide shows how to run a Sysand index from a private GitHub
repository. This allows you to host an index without needing to ask your IT
department to set up a server for you, while also allowing simple access control
through GitHub's access management.

Sensmetry has a [public GitHub repository][github_repo] implementing this
approach. It is licensed under the Creative Commons Zero license and can be
freely forked and used in any setting. The repository README contains a detailed
explanation of how to set up such a repository and use it.

## GitLab

This part of the guide shows how to run a Sysand index from a private GitLab
Pages instance. This allows you to host an index without needing to ask your IT
department to set up a server for you, while also allowing simple access control
through GitLab's access management.

Sensmetry has a [public GitLab repository][gitlab_repo] implementing this
approach. It is licensed under the Creative Commons Zero license and can be
freely forked and used in any setting. The repository README contains a detailed
explanation of how to set up such a repository and use it.

[sysand_index]: https://beta.sysand.org/
[github_repo]: https://github.com/sensmetry/sysand-private-index
[gitlab_repo]: https://gitlab.com/sensmetry/public/sysand-private-index
