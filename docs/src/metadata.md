# Project information and metadata

Each project is defined by interchange project information (`.project.json`)
and metadata (`.meta.json`) files in the root directory of the project.

Contents of `.project.json` and `.meta.json` files are specified in [KerML
specification](https://www.omg.org/spec/KerML/1.0), chapter 10.3. Sysand imposes
additional requirements for some fields, as documented below. These extra
requirements are imposed to achieve better interoperability between different
tools, user convenience and to have more structured project information.

## Fields of `.project.json`

### `license`

Specification does not have any requirements for license format. Sysand
encourages the use of [SPDX License Expressions][spdx] to specify licenses.
By default Sysand rejects `license` strings that are not valid SPDX License
Expressions. Examples of valid expressions:

- `MIT` - MIT license
- `MIT OR Apache-2.0` - MIT and/or Apache-2.0 license applies, chosen by the
  user
- `LicenseRef-My-License` - a custom license (`LicenseRef-` part is mandatory
  in this case). This should be used when the license used is not in
  [SPDX License list][spdxlist].

See the [specification][spdx] and [license list][spdxlist] for more information.
If a project's license is not given, it is assumed that the project cannot be
used under any license without owner's explicit permission.

As specified by [REUSE project][reuse], license files should be included at
the top level directory of the project inside `LICENSES` subdirectory. License
file name should match the `license` field's value with `.txt` appended, except
when using composite expressions (those containing `OR`, `AND` or `WITH`). In
those cases include all the license/exception files mentioned in the expression.
For the complete set of SPDX license and exception files, see the [SPDX GitHub
repository][llist]. Custom licenses (such as `LicenseRef-MyCustom`) result in
license file name `LicenseRef-MyCustom.txt`.

Each file should have its license specified at the top in a comment. See [REUSE
spec][reuseheader] for details.

[spdx]: https://spdx.github.io/spdx-spec/v3.0.1/annexes/spdx-license-expressions/
[spdxlist]: https://spdx.org/licenses/
[reuse]: https://reuse.software/
[llist]: https://github.com/spdx/license-list-data/tree/main/text
[reuseheader]: https://reuse.software/faq/#step-2

### `version`

Specification does not impose any constraints on how the version should
look like, it only recommends to use [Semantic Versioning (SemVer)][semver].
Sysand strongly encourages users to use SemVer and by
default does not accept non-SemVer 2.0.0 version strings in its commands.

### `usage`

An array identifying all the projects used by the current project (commonly
called dependencies in programming contexts).

Each project is identified by an [IRI (Internationalized Resource Identifier)][iri].
IRI is a superset of [URI (Uniform Resource Identifier)][uri], which is a superset
of [URL (Uniform Resource Locator)][url], meaning that every URL or URI is also
an IRI. For this reason, elsewhere in this documentation IRI, URI and URL are
used interchangeably and mean IRI, unless specified otherwise.

IRIs are not required to be resolvable (i.e. be URLs that have a well-defined
way of obtaining the identified resource). So Sysand tries to obtain each
project using a variety of methods. The first method that successfully obtains
the project is chosen. The order in which different methods are tried is
unspecified and may change in the future.

Sysand currently supports (i.e. knows how to obtain) these IRI schemes:

- `http`/`https`: can point to either a KPAR file or to a "directory"
  containing `.project.json`/`.meta.json`.
- `file`: can point to either a KPAR file or to a directory containing
  the project.
- `ssh`: note that currently only git repositories are supported for this type.
  SSH repository URLs supported by git have to be translated to use standard ssh
  syntax to be accepted by sysand. For example:

  `git@github.com:myuser/myrepo.git`

  translated into standard syntax becomes

  `ssh://git@github.com:22/myuser/myrepo.git`

  See [git URL documentation][git_url] for
  details.

- `git+file`/`git+http`/`git+https`/`git+ssh`: same as non-prefixed protocols,
  but explicitly identify that the destination is a git repository and should
  be treated as such. This is a way to force Sysand to only use git resolver
  to obtain the project.
- `urn:kpar`: this is by convention used by all projects in the
  [Sysand index](https://beta.sysand.org/), but otherwise has no special meaning

Projects in HTTP(S) indexes can use any IRI schemes (including the ones mentioned
above), since scheme is not taken into account when trying to obtain a project
from an index. It is recommended to use `urn` scheme for projects in an index
to avoid confusion over how to obtain it.

[iri]: https://en.wikipedia.org/wiki/Internationalized_Resource_Identifier
[uri]: https://en.wikipedia.org/wiki/Uniform_Resource_Identifier
[url]: https://en.wikipedia.org/wiki/URL
[git_url]: https://git-scm.com/docs/git-fetch.html#_git_urls

### `versionConstraint`

`versionConstraint` field can be used to constrain the allowed versions
of a specific project within a `usage`.

Version constraints use the same syntax as [Rust's Cargo][cargo-semver].
The usage resolution version selection mechanism is only implemented
for semantic version constraints and semantic versions of used
projects. Sysand will not be able to correctly (or at all) select
versions for usages that do not adhere to SemVer.

See below for details.

## Version constraint syntax

A version constraint consists of one or more version comparators,
separated by commas. Each version comparator consists of
an operator and a version. Version is a SemVer with possibly omitted
trailing components, e.g. `1.0.0`, `2.3`, `5`, `2.3.4-beta` are all
valid in a comparator. The missing components are ignored
when matching against a version. Note that versions can only be
truncated in version constraints, but not in any other context.
In order for the version to satisfy a constraint, it must match all
of the comparators in the constraint.

Version comparison operators are listed below. In the examples,
`:=` denotes equivalence between expressions.

### No operator

A bare version specifier, such as `1.2.3`, `2`, `3.1`.
It is exactly equivalent to a caret (`^`) operator.

### Caret operator

Caret operator (`^`) allows [SemVer] compatible updates.

Leaving off the caret is a simplified equivalent syntax.
It is recommended to use the caret syntax for added clarity.

Versions are considered compatible if their left-most non-zero
major/minor/patch component is the same. This is different from
SemVer which considers [all pre-1.0.0 packages to be incompatible][semver-0].

Examples:

```text
^1.2.3  := 1.2.3 := >=1.2.3, <2.0.0
^1.2    := 1.2   := >=1.2.0, <2.0.0
^1      := 1     := >=1.0.0, <2.0.0
^0.2.3  := 0.2.3 := >=0.2.3, <0.3.0
^0.2    := 0.2   := >=0.2.0, <0.3.0
^0.0.3  := 0.0.3 := >=0.0.3, <0.0.4
^0.0    := 0.0   := >=0.0.0, <0.1.0
^0      := 0     := >=0.0.0, <1.0.0
```

### Tilde operator

Tilde operator (`~`) specifies a minimal version with some ability to update.
If a major, minor, and patch version or only a major and minor
version is specified, only patch-level changes are allowed. If only a major
version is given, then minor- and patch-level changes are allowed.

Examples:

```text
~1.2.3  := >=1.2.3, <1.3.0
~1.2    := >=1.2.0, <1.3.0
~1      := >=1.0.0, <2.0.0
```

### Wildcard operator

Wildcard operator (`*`) allows for any version where the wildcard is
positioned.

Examples:

```text
*     := >=0.0.0
1.*   := >=1.0.0, <2.0.0
1.2.* := >=1.2.0, <1.3.0
```

### Equals operator

Equals operator (`=`) means the exact version is required.
Since the version in a comparator may be partial, only the
parts specified are required to match exactly.

Examples:

```text
=1.2.3 := >=1.2.3, <1.2.4
=1.2   := >=1.2.0, <1.3.0
=1     := >=1.0.0, <2.0.0
```

### Comparison operators

Comparison operators (`<`, `<=`, `>`, `>=`) are the most basic,
as all the other comparators can be equivalently translated
to (possibly multiple) comparison comparators. If only one
comparison operator is given, the allowed versions range has
no opposite end.

Examples:

```text
>=1.2.0
>1      := >=2.0.0
<2      :=  <2.0.0
<=1.5   :=  <1.6.0
```

### Multiple comparators

As mentioned and shown in the examples above, multiple version comparators
can be used in a single constraint when separated with a comma, e.g.,
`>=1.2, <1.5`.
All comparators in a constraint must be satisfied, so a non-overlapping
constraint like `<1.2, ^1.2.2` is unsatisfiable.

### Pre-releases

Version constraints exclude pre-release versions, such as `1.0.0-alpha`,
unless specifically asked for, i.e. the exact same version with any
pre-release tag must be used in a comparator in order for that
comparator to try to match the pre-release versions.

Pre-release versions are considered to be lower than regular versions in
[SemVer spec][semver-sort]. Comparators specifying a pre-release tag
can also match regular versions.

Examples:

- constraint `1.0` will not be satisfied by version `1.0.0-alpha`
- constraint `>=1.0.0-beta.2, <2.0.0-alpha.5` will be satisfied by
  version `1.2.3-rc.2`
- constraint `>=5.4.2-beta1` will be satisfied by versions `5.4.2`, `6.2.0`
  and `5.4.2-rc`, but not `5.4.3-alpha`
- constraint `^1.2.3-rc` will be satisfied by version `1.2.3`

### Version metadata

Version metadata, such as `1.0.0+21AF26D3`, is ignored and
should not be used in version requirements.

### More information

See [Cargo docs][cargo-semver], [`semver` crate docs][semver-op]
(`semver` crate is used by both Sysand and Cargo for version selection)
and [SemVer specification][semver] for more detailed information
regarding version constraints and semantic versioning.

[semver]: https://semver.org/
[semver-0]: https://semver.org/spec/v2.0.0.html#spec-item-4
[cargo-semver]: https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#version-requirement-syntax
[semver-op]: https://docs.rs/semver/latest/semver/enum.Op.html
[semver-sort]: https://semver.org/spec/v2.0.0.html#spec-item-11
