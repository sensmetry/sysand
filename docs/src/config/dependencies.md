# Dependencies

Sometimes you may wish to use a project that isn't resolvable through an
available index or you want to override the dependency resolution for other
reasons. In any case you can do this by adding the appropriate IRI and `Source`
to a `project` entry in the `sysand.toml` configuration file at the root of
your project. This follows the same structure as found in the lockfile, where
`identifiers` are given as a list of IRI:s and `sources` are a list of sources.
A project may have multiple identifiers in case it is referred to differently
by different projects, and multiple sources where the additional ones after the
first serve as backups in case the previous ones fail to resolve. Note that
these should be sources of the exact same project as determined by it's
checksum, as otherwise you are likely to run into problems when syncing against
a lockfile.

Below we describe how add overriding sources directly to the configuration
file, but it is also possible to do through the command line interface with the
[`sysand add`](../commands/add.md) command.

## Local projects

To specify the source of a project that you have locally in a directory
`./path/to/project` by the identifier `urn:kpar:my-project`, is done by adding
the following entry to your `sysand.toml`.

```toml
[[project]]
identifiers = [
    "urn:kpar:my-project",
]
sources = [
    { src_path = "path/to/project" },
]
```

Note that the path to the project is given by path that is relative to the root
of your project.

## Local editable projects

Normally when you add a project as a usage, `sysand` will copy and install it,
so any changes made to the project after will not affect the installed project.
For local projects you also have the option to add them as "editable" usages,
meaning the project won't be copied and will instead just be referred to where
it is originally located. A local project is specified as editable in
`sysand.toml` by adding

```toml
[[project]]
identifiers = [
    "urn:kpar:my-project",
]
sources = [
    { editable = "path/to/project" },
]
```

## Local KPARs

If you have a project locally available as a compressed KPAR this can be identified
by `urn:kpar:my-kpar-project` by adding

```toml
[[project]]
identifiers = [
    "urn:kpar:my-kpar-project",
]
sources = [
    { kpar_path = "path/to/project.kpar" },
]
```

to your `sysand.toml`.

## Remote projects and KPARs

To specify a remote project as a source, add

```toml
[[project]]
identifiers = [
    "urn:kpar:remote-project",
]
sources = [
    { remote_src = "https://www.example.com/path/to/project" },
]
```

to your `sysand.toml`, or for a remote KPAR you add

```toml
[[project]]
identifiers = [
    "urn:kpar:remote-kpar-project",
]
sources = [
    { remote_kpar = "https://www.example.com/path/to/project.kpar" },
]
```
