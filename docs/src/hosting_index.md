# Self-hosting a Project Index

## Introduction

This section provides instructions on how to use Sysand CLI to host a private
Sysand project index. The guide describes three ways to run the index:

- [Local Machine](#local-machine) -- running a HTTP file server on your machine
- [GitHub](#GitHub) -- using a GitHub repository
- [GitLab](#GitLab) -- using GitLab Pages

> [!note]
> This guide is only concerned about hosting the package files in the
> structure that Sysand CLI can understand. Hosting of a front-end website
> (such as [beta.sysand.org][sysand_index]) is not a part of this guide.

> [!warning]
> Sysand is in active development. The structure of indexes and
> `sysand_env` folders **can and will change** with future updates of Sysand. As
> long as Sysand is on version 0.x.y, we cannot guarantee backwards
> compatibility between different Sysand versions and indexes created using
> different Sysand versions.

## Local Machine

This part of the guide focuses on running a Sysand index on your local machine
for testing purposes, but the general approach applies to a more sophisticated
production hosting as well. Get in touch with your IT administrator to get this
running on your company's infrastructure.

The easiest way to host a project index from which to install packages is to
expose a `sysand_env` over HTTP, since indexes and `sysand_env` share the same
structure that is understood by Sysand.

### Create `sysand_env`

First, use the Sysand CLI to create a Sysand environment:

```sh
sysand env
```

This will create a `sysand_env/` folder in your current directory.

### Add Packages to the Environment

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
> point to another resource, like the ones starting with `http(s)`, `file` or
> `ssh`.

By default, this command also installs all usages (dependencies) of
`my_project`. You can use the `--no-deps` CLI flag to only install the package
without dependencies.

If you want to host multiple versions of the same package in your repository,
you also need to use the `--allow-multiple` CLI flag in the `sysand env install`
command.

Repeat this step for as many times as you have packages (and their versions),
giving a unique IRI for each different package.

### Start an HTTP server

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

### Sysand Client Setup

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

Sensmetry have a [public GitHub repository][github_repo] implementing this
approach. It is licensed under the Creative Commons Zero license and can be
freely forked and used in any setting.

### Repository Contents

There are two important branches in the repository:

- `main` branch -- This is the main branch for human interactions. This branch
  contains the `packages` folder that holds the `.kpar` files of all the
  projects that need to be shared through the index.
- `index` branch -- This branch is not supposed to be interacted with by humans.
  This branch is exposed to the Sysand client as the index. It contains an
  auto-generated file structure and always uses `git reset --hard` and `git push
  --force` to avoid exploding the size of the git repository. Ideally, branch
  rules would be set up that would only allow the automated bot to make changes
  to the `index` branch.

Other branches can also be used, e.g. to allow reviews of the projects before
publishing them. However, they should only target the `main` branch, and not
`index`. Additionally, currently the GitHub Workflow is set up to run only on
the `main` branch.

### GitHub Workflow

A GitHub Workflow can be found in the `.github/workflows/ci.yml` file on the
`main` branch. This workflow is triggered on every commit to the `main` branch,
at which point it:

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

You should now be able to access the package index through
`https://raw.githubusercontent.com/OWNER/REPO/refs/heads/index/`, where `OWNER`
and `REPO` is specific to where you created the project and how you named it. To
test it, create a new SysML v2 project in another directory by following the
[User Guide](tutorial.md).

Before you can use the index when adding usages, you need to tell Sysand how to
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
to point to your private package index instead of the public
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

Sensmetry have a [public GitLab repository][gitlab_repo] implementing this
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

A CI/CD Pipeline can be found in the `.gitlab-ci.yml` file on the `main` branch.
This pipeline is triggered on every commit to the `main` branch, at which point
it:

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

Before you can use the index when adding usages, you need to tell Sysand how to
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
[github_pat]: https://github.com/settings/personal-access-tokens
[gitlab_repo]: https://gitlab.com/sensmetry/public/sysand-private-index
[gitlab_pat]: https://gitlab.com/-/user_settings/personal_access_tokens
