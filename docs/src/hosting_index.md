# Self-hosting a Project Index

## Introduction

This section provides instructions on how to use Sysand CLI to host a private
Sysand project index. The guide describes three ways to run the index:

- [Local Machine](#local-machine) -- running a HTTP file server on your machine
- [GitHub](#GitHub) -- using a GitHub repository
- [GitLab](#GitLab) -- using GitLab Pages

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

> [!note]
> Since GitHub Pages do not support authentication using personal access
> tokens (in contrast to GitLab Pages), this guide presents a workaround that
> uses access to raw file contents through `raw.githubusercontent.com`. This
> solution might not be ideal for hosting big indices, as GitHub can rate-limit
> access to `raw.githubusercontent.com` without prior notice.

Sensmetry has a [public GitHub repository][github_repo] implementing this
approach. It is licensed under the Creative Commons Zero license and can be
freely forked and used in any setting.

### Repository Contents

There are two important branches in the repository:

- `main` branch -- This is the main branch for human interactions. This branch
  contains the `packages` folder that holds the `.kpar` files of all the
  projects that need to be shared through the index.
- `index` branch -- This branch is not supposed to be interacted with by humans.
  This branch is exposed to the Sysand client as the index. It contains an
  auto-generated file structure and always uses `git reset --hard` and
  `git push --force` to avoid exploding the size of the git repository. Ideally,
  branch rules should be set up that would only allow the automated bot to make
  changes to the `index` branch.

Other branches can also be used, e.g. to allow reviews of the projects before
publishing them. However, they should only target the `main` branch, and not
`index`. Additionally, currently the GitHub Workflow is set up to run only on
the `main` branch.

### GitHub Workflow

A GitHub Workflow can be found in the
[`.github/workflows/ci.yml`][github_workflow] file on the `main` branch. This
workflow is triggered on every commit to the `main` branch, at which point it:

1. Installs Sysand client
2. Creates a Sysand environment, and installs all `.kpar` projects from the
   `packages` as described in the [Add Packages to the
   Environment](#add-packages-to-the-environment) section above
3. Resets the `index` branch to the initial commit using `git reset --hard`
4. Takes the environment created in Step 2 and creates a `git commit`
5. Pushes the new commit to the `index` branch with `git push --force`

> [!note]
> During step 2, the Workflow generates a `urn` for the project from the
> contents of the `name` field in `.project.json` of the project. Therefore, our
> strong suggestion is to keep the `name` only contain lower-case ASCII
> characters with no spaces. However, the Workflow can be adjusted to allow for
> other naming conventions.

### Sysand Client Setup

You should now be able to access the project index through
`https://raw.githubusercontent.com/OWNER/REPO/refs/heads/index/`, where `OWNER`
and `REPO` is specific to where you created the project and how you named it. To
test it, create a new SysML v2 project in another directory by following the
[User Guide](tutorial.md).

Before you can use the index for adding usages, you need to tell Sysand how to
authenticate with your index. You can do this by setting the following
environment variables. See [Authentication](authentication.md) for details.

- `SYSAND_CRED_GITHUB` to the value of
  `https://raw.githubusercontent.com/OWNER/REPO/refs/heads/index/**`. Do not
  forget to replace `OWNER` and `REPO` to your values. Note: the `**` ending is
  important.
- `SYSAND_CRED_GITHUB_BEARER_TOKEN` to the value of the [GitHub Personal Access
  Token][github_pat]. We recommend using a fine-grained token scoped to this
  index repository only, and with only `Contents` read-only permissions.

Now, when adding a new usage to the project, use the `--index` argument
to point to your private project index instead of the public
[beta.sysand.org][sysand_index], for example:

```sh
sysand add urn:kpar:my_project --index https://raw.githubusercontent.com/OWNER/REPO/refs/heads/index/
```

An alternative to `--index` argument is defining the custom index in
`sysand.toml`. See [Indexes](config/indexes.md) for details.

## GitLab

This part of the guide shows how to run a Sysand index from a private GitLab
Pages instance. This allows you to host an index without needing to ask your IT
department to set up a server for you, while also allowing simple access control
through GitLab's access management.

Sensmetry has a [public GitLab repository][gitlab_repo] implementing this
approach. It is licensed under the Creative Commons Zero license and can be
freely forked and used in any setting.

### Repository Contents

The repository is quite simple, containing a `main` branch that holds the
`.kpar` files of all the projects that need to be shared through the index and a
CI/CD pipeline definition.

Other branches can also be used, e.g. to allow reviews of the projects before
publishing them. Currently, the pipeline builds the index on all branches, but
publishes the index only on the `main` branch.

### CI/CD Pipeline

A CI/CD Pipeline can be found in the [`.gitlab-ci.yml`][gitlab_pipeline] file on
the `main` branch. This pipeline is triggered on every commit to the `main`
branch, at which point it:

1. Installs Sysand client
2. Creates a Sysand environment, and installs all `.kpar` projects from the
   `packages` as described in the [Add Packages to the
   Environment](#add-packages-to-the-environment) section above
3. Uses `pages` job to deploy the generated Sysand environment to GitLab Pages.

> [!note]
> During step 2, the pipeline generates a `urn` for the project from the
> contents of the `name` field in `.project.json` of the project. Therefore, our
> strong suggestion is to keep the `name` only contain lower-case ASCII
> characters with no spaces. However, the pipeline can be adjusted to allow for
> other naming conventions.

### Sysand Client Setup

You should now be able to access the package index through
`https://GITLAB-ASSIGNED-DOMAIN.gitlab.io` or your custom domain. To test it,
create a new SysML v2 project in another directory by following the [User
Guide](tutorial.md).

Before you can use the index for adding usages, you need to tell Sysand how to
authenticate with your index. You can do this by setting the following
environment variables. See [Authentication](authentication.md) for details.

- `SYSAND_CRED_GITLAB` to the value of
  `https://GITLAB-ASSIGNED-DOMAIN.gitlab.io/**`. Do not forget to replace
  `GITLAB-ASSIGNED-DOMAIN` with your value. Note: the `**` ending is important.
- `SYSAND_CRED_GITLAB_BEARER_TOKEN` to the value of the [GitLab Personal Access
  Token][gitlab_pat]. We recommend using a token with only `read-api` scope.

Now, when adding a new usage to the project, use the `--index` argument
to point to your private package index instead of the public
[beta.sysand.org][sysand_index], for example:

```sh
sysand add urn:kpar:my_project --index https://GITLAB-ASSIGNED-DOMAIN.gitlab.io
```

An alternative to `--index` argument is defining the custom index in
`sysand.toml`. See [Indexes](config/indexes.md) for details.

[sysand_index]: https://beta.sysand.org/
[github_repo]: https://github.com/sensmetry/sysand-private-index
[github_workflow]: https://github.com/sensmetry/sysand-private-index/blob/main/.github/workflows/ci.yml
[github_pat]: https://github.com/settings/personal-access-tokens
[gitlab_repo]: https://gitlab.com/sensmetry/public/sysand-private-index
[gitlab_pipeline]: https://gitlab.com/sensmetry/public/sysand-private-index/-/blob/main/.gitlab-ci.yml?ref_type=heads
[gitlab_pat]: https://gitlab.com/-/user_settings/personal_access_tokens
