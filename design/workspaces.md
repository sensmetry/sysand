# Sysand client workspaces

## Config

Matching cargo/uv: a `[workspace]` table in a `model.toml` file
(hereafter: workspace config) defines a workspace. Its containing folder
is called a workspace root. A `[project]` table in the same file format
defines a project (see model-toml.md). Exactly one of the two appears in
a file: they never appear together, as a workspace root must not contain
a project (see below) and a member that was also a workspace root would
be a nested workspace, which is rejected (see Non-goals for v1); and a
`model.toml` with neither is an error (see model-toml.md). Like every
`model.toml`, a workspace config starts with `manifest-version = 1` (see
model-toml.md, Manifest version). A workspace config has no top-level
`inline-usages`: that array belongs to a project, and a workspace's
common usages are declared under `[workspace]` (see Common usages), so
one at the top level of a workspace config is an error.

The workspace is discovered by walking up from cwd until a `model.toml`
with a `[workspace]` table is encountered; that folder is the candidate
workspace root. The walk does not stop there: it continues to the
filesystem root (there is no `.git` bound), and a second `[workspace]`
table found above the first means the candidate is itself inside another
workspace. That is allowed in exactly one case: the outer workspace
lists the candidate's directory, or a directory above it, in `exclude`
(see Included projects). Then the candidate is confirmed as the root and
the walk stops; whether the outer workspace is itself nested is checked
when a command runs inside it. Otherwise the two workspaces are nested,
which is rejected (see Non-goals for v1).

Nesting is only ever looked for on that upward walk. Subtrees are not
scanned, so a `model.toml` with a `[workspace]` table somewhere below the
workspace root is not detected and is not by itself an error; it is
rejected when a command is run from inside it, which is when it appears
on the walk. A member cannot be such a root in any case, since the two
tables never share a file and a member must define a project.

Once the root is known, the walk classifies the project the command
started in, if any: it is a member (listed in `members`), it is
standalone (under an `exclude`d directory), or it is neither, which is a
hard error (see Included projects).

### Included projects

Explicit. A list of directories, each containing a single project.

Projects included in the workspace are called workspace members.
Membership is never declared by the member: workspace config defines it
by the array `members` containing explicit paths (globs will be supported
in the future). Paths can go arbitrarily deep, but must not go above the
workspace root.

```toml
manifest-version = 1

[workspace]
members = ["folder1", "folder2/subfolder1"]
exclude = ["vendor"]
```

Rules on `members`:
- every entry must resolve to a project that uses the `model.toml`
  project definition (see Member requirements); an entry that resolves
  to a json project, to a directory with both manifests, or to no
  project at all is an error
- no entry may lie inside another entry's directory: nested members are
  forbidden for now (lifting the rule later would be additive), and two
  entries where one path is an ancestor directory of the other are
  rejected at read time, in the same error family as a path that escapes
  the root
- the same project listed twice is rejected

`exclude` is a list of directories relative to the workspace root,
following the same path rules as `members` (relative, below the root,
never the root itself, and written in the same form). Anything under one of them is outside the
workspace: a project there is
a standalone project with its own lockfile, `.sysand` and `output`, and a
workspace root there is a legitimate, separate workspace (see Config).
This is the only kind of non-member project that may sit under a
workspace root.

A project found under the workspace root that is neither a member nor
under an excluded directory is a hard error, never a guess: "the project
at `<path>` lies under the workspace at `<root>` but is neither listed in
`workspace.members` nor under `workspace.exclude`; add it to one of the
two". The error fires on the upward walk from the project's directory,
never on a downward scan, so `sysand build` at the root succeeds while
the same command run inside an unlisted project errors. The rule makes
"which workspace owns this project" decidable by construction, and it
keeps such a project from being silently unusable (it has no lockfile,
environment or output directory of its own).

Inheritance markers (`<key>.workspace`, see Explicit inheritance) are a
separate matter: they do not establish membership, but a project carrying
one cannot be read outside a workspace.

Workspace root must not contain a project. No technical reason, just
nudges people towards a nicer workspace structure.

Enforced in two places:
- the `model.toml` that defines the workspace must not also contain a
  `[project]` table
- any `members` entry that resolves to the workspace root itself (`"."`,
  `""`, `"./"`, `"folder1/.."`, ...) is rejected; this complements the
  rule above that paths must not go above the workspace root

Consequences:
- every workspace is what Cargo calls a virtual workspace, so no
  terminology is needed to distinguish workspace kinds
- the workspace root is never a member, so commands run from the
  workspace root always target the whole workspace (see Command scopes)

### Stray per-member state

A member never has state of its own: the lockfile, the `.sysand`
directory and the `output` directory all belong to the workspace root
(see Locking, Syncing, Build output). A member directory that still
contains a `sysand-lock.toml`, a `.sysand/` or an `output/` -- typically
a standalone project that has just been listed in `members` -- is an
error: "member `<path>` has its own `sysand-lock.toml`; a workspace has
one lockfile at `<root>`; remove the member's copy". The check runs
whenever members are loaded from the root, or the starting project is
classified as a member on the upward walk: three existence tests, no
content read. Starting with an error keeps the option of relaxing it
later to a warning or to automatic removal without breaking anyone.
Standalone projects under an `exclude`d directory keep their state
legitimately; the check applies to members only.

### Common metadata

Note: these are the properties that may be inherited by workspace
members.

- publisher
- version
- license
- urls (table) (whole table or individual keys can be inherited):
  - website
  - repository
  - documentation
- topics
- metamodel
- maintainers

The other `[project]` keys are deliberately absent:
- `name` and `description` identify one project
- `sources` (see model-toml.md) is per-project only, cannot appear in
  `[workspace]` (neither in the default group nor in a named one), and
  cannot be inherited, since the file layout of one project says nothing
  about another's. Because it is the key people are most likely to try,
  a `sources` in `[workspace]` or in a group is an error that says so,
  rather than an unknown-key warning
- `stdlib` (see model-toml.md, Standard library) is a per-project
  identifier; in `[workspace]` or a group it is an unknown key like any
  other (a warning)
- `manifest-version` is not a `[project]` key at all: every file carries
  its own (see model-toml.md, Manifest version)

`metamodel` doubles as a schema selector (see model-toml.md, Metamodel),
and inheriting it is what lets a group stand for one metamodel release:
a group whose members inherit `metamodel` is single-metamodel by
construction. There is no workspace-level metamodel. Members with
different metamodels, OMG and Syside ones included, may share a
workspace; they use per-project `metamodel` values or one group per
metamodel, and the only constraint between them is the dependency
direction rule (an OMG-metamodel project must not depend on a
Syside-metamodel project), which is checked per usage during resolution
(see Dependency resolution).

### Common usages

Specified the same way as in `model.toml` (see model-toml.md), as
`[[workspace.inline-usages]]` for the default group and
`[[workspace.group.<group-name>.inline-usages]]` for a named group.

A `dir` or `kpar-path` in a group entry is relative to the directory of
the `model.toml` that defines the workspace, i.e. the workspace root,
whichever member opts into it. A member's own `[[inline-usages]]` stay
relative to the member.

Members opt in per usage, not for the array as a whole: an entry that
gives only the identity plus a `workspace` marker takes the matching
group entry whole. `workspace = true` selects the default group,
`workspace = "group-name"` a named one, as for metadata.

Entries are matched by publisher and name, compared literally (see
model-toml.md, Normalization). The group entry is used as-is; the member
cannot change any of its fields. A member may declare its own usages
alongside inherited ones.

An opt-in must find its entry: a marker naming a group that does not
exist, or a group with no entry of that identity, is an error, as for
inherited metadata. Within any one array -- a project's `inline-usages`,
or a single group's -- two entries of the same identity are rejected as
duplicates (see model-toml.md, Normalization), which includes an
inherited entry colliding with one the member declares itself. Across
groups the same identity is expected: that is what groups are for.

```toml
[[inline-usages]]
publisher = "acme"
name = "foo"
workspace = true

[[inline-usages]]
publisher = "acme"
name = "bar"
workspace = "legacy"

[[inline-usages]]
publisher = "acme"
name = "own"
dir = "../own"
```

The standard library is the motivating case for groups: projects that
target different releases of it, or different languages, take their
standard-library usages from different groups, each with its own
`metamodel`:

```toml
[workspace.group.kerml]
metamodel = "https://www.omg.org/spec/KerML/20250201"

[[workspace.group.kerml.inline-usages]]
publisher = "OMG"
name = "Kernel Semantic Library"
version = "^1.0.0"

[workspace.group.sysml]
metamodel = "https://www.omg.org/spec/SysML/20250201"

[[workspace.group.sysml.inline-usages]]
publisher = "OMG"
name = "SysML Systems Library"
version = "^2.0.0"
```

A member whose standard-library release differs from the release its
own `metamodel` names gets the release warning of model-toml.md,
Metamodel.


## Locking

Always a single lockfile, `sysand-lock.toml`, for the whole workspace in
the workspace root. It records every project of the solved graph by the
identifier `pkg:sysand/<publisher>/<name>` (see model-toml.md,
Normalization), members included.

A `dir` usage is recorded as *editable*: the lockfile keeps the path and
no checksum, whether the usage resolves to a member, to a project outside
the workspace root, or -- for a standalone project -- to any directory at
all, so that editing a source project never churns the lockfile.
Checksums stay with the kinds that name immutable artifacts: `kpar-path`,
index and `url` usages. The `--as-local-src` behaviour remains available
to json projects only; a toml `sysand add` needs no choice for `dir`.

The lockfile is consistent with the workspace when it matches all
members' manifests: an edit to any member's `model.toml` re-solves the
whole workspace, exactly as a member's `Cargo.toml` edit reloads a Cargo
workspace. Any tool that watches for changes treats a `model.toml` or
`sysand-lock.toml` change anywhere under the root as one reconcile of the
workspace.


## Syncing

Always a single `.sysand` directory in workspace root. Standard-library
entries are provided by the tooling unless `--include-std` is passed
(see model-toml.md, Standard library): a lockfile that pins a
standard-library release the tooling does not provide is an error on
`sync`, and `--include-std` is the way out.


## Build output

A single `output` directory in workspace root, the directory Sysand
uses today; a member never has one of its own (see Stray per-member
state). Currently the only artifacts produced are the KPARs, one per
member, named `<name>-<version>.kpar` as for a standalone project (see
model-toml.md, TOML to KPAR).

A workspace builds every member, in `members` order, and a member that
fails does not stop the others: a build reads a target's `version` from
its manifest and never from a built artifact, so no member's build needs
another's. `sysand publish` takes the members in dependency order
instead: a topological sort of the members' `dir` usages of each other,
with the projects of an allowed cycle in `members` order. A dependency
cycle is a hard error listing the cycle, unless every project on it
carries the `stdlib` key (see model-toml.md, Circular dependencies).
`sysand build --allow-path-usage` applies to every member built.


## Dependency resolution

A workspace is solved as one unit: every member's usages, after the
workspace opt-ins have been replaced by their group entries, go into one
solve that picks one version of each dependency for the whole workspace.
Minor and patch differences between members' constraints are unified
the way any semver solve unifies them. If two members' constraints admit
no common version, that is an error naming both members, e.g. "`a`
requires `pub/lib ^1` and `b` requires `pub/lib ^2`; a workspace uses
one version of each dependency".

Two versions of one dependency side by side (different major versions,
as Cargo allows) are not supported in v1: v1 has only `inline-usages`
and no way to give a dependency a second name, and the lockfile, the
`.sysand` environment and the symbol namespace all hold one version per
identity. A later version may add per-usage aliases and, with them,
Cargo's approach: all majors that members ask for are included, one
version within each major.

Three checks run over every edge of the solved graph, before the
lockfile is written and again when it is validated on `sync` and `build`
(see model-toml.md, Metamodel and Semver dependencies): the metamodel
direction rule (error), the release mismatch rule (warning), and the
semver rule, whose error names the usage chain from the member to the
offending project.

### Intra-workspace usages

Intra-workspace usages (i.e. one member depends on another) are
explicit and specified as relative path source usages (`dir`, relative
to the member, or a group entry relative to the root). Such a usage
resolves to the member itself, never to a copy of it from an index, and
is recorded as editable (see Locking). A `dir` usage from a member to a
project that lies under the root without being a member is the
unlisted-project error of Included projects.

### Source overrides

Source overrides are specified in `sysand.toml`, the existing Sysand
config file format, and not in `model.toml`. Moving them into
`model.toml` is deferred to after v1.

For v1 `sysand.toml` remains supported, but a project-local one must be
at the workspace root: overrides apply workspace-wide, which avoids
significant resolver complexity, as a member cannot override sources for
itself alone. A `sysand.toml` anywhere else under the workspace is
ignored as a whole -- its `[[index]]` configuration included -- with a
warning on any operation touching the overrides or the indexes. The user-level
`sysand.toml` (under `$XDG_CONFIG_HOME/sysand/`) keeps merging beneath
the workspace root's file, as it merges beneath a standalone project's
today.


## Inheritance and overrides

What and how members inherit from workspace config.

Overrides are only relevant if there is implicit inheritance.

There will be no implicit property inheritance from workspaces,
and therefore no configuration overrides.

### Explicit inheritance

Named groups can be used to assign different values of keys that can be
inherited by projects. The default group is implicit: its keys are
written directly in `[workspace]`, alongside `members` and `exclude`.
Keys for other groups are declared in `[workspace.group.<group-name>]`
tables, where `<group-name>` is a bare TOML key: letters, digits, `-`
and `_`.

A marker must find a value: naming a group that does not exist, or one
that does not define the key, is an error, whether or not the key is
required in `[project]`.

In every project, to inherit a key from the default group,
`<key>.workspace = true` is used. To inherit from a non-default group,
`<key>.workspace = "group-name"`. There is no "inherit nothing" marker:
`<key>.workspace = false` is an error, since a key that is not inherited
simply carries its own value. This form is allowed wherever the key
itself would be, required keys included: `publisher.workspace = true`
and `version.workspace = true` satisfy their requirement in `[project]`.
The exceptions are `sources` and `stdlib`, which are not inheritable
(see Common metadata): `sources` is always written out in full by every
member.

Markers resolve only against the workspace the project is a *member* of.
A project that is not a member is outside every workspace regardless of
where its directory sits, so a `dir` usage that resolves to a
marker-bearing project which is not a member of any workspace is an
error: "`<key>` inherits from <group>, but the project is not a member
of any workspace". A `dir` usage to a marker-free non-member reads it as
an ordinary standalone project.

Usages are not inherited this way: they use the per-entry opt-in
described in Common usages.

Inheritance is whole-key: a member either inherits a key or writes it out
in full, never both. Nothing is merged or layered, which is what keeps
"no implicit inheritance, and therefore no overrides" true of explicit
inheritance as well.

It follows that inheritance cannot overlap: when a key is inherited from
a table and the key's value is also a table, inheritance cannot be
specified for any of the inner table's keys, nor can the inner key be
given a value of its own. The same holds for arrays: an inherited
`topics` cannot be extended by a member. Example:
```toml
urls.workspace = true
urls.website.workspace = true # invalid
urls.website.workspace = "some-group" # invalid
urls.website = "https://example.com" # invalid
```

## Member requirements

- Must use the `model.toml` project definition; a `members` entry that
  resolves to a json project is an error (see Included projects)
- Must have no lockfile, `.sysand` or `output` of its own (see Stray
  per-member state)
- Must only depend (including transitively) on projects using semver
  (note that this does not require constraints to be present);
  this allows simplifying the dependency solver.
  Since "including transitively" cannot be established from a manifest
  alone, this is checked during resolution: a usage that resolves to a
  project whose version is not semver is an error naming the usage chain
  from the member to that project and quoting the version (see
  model-toml.md, Semver dependencies). Kinds that carry no version
  constraint (remote-kpar) and json targets are therefore not forbidden
  up front; they fail only if what they resolve to is not semver


## Command scopes

Commands follow Cargo's per-command defaults. When run from the
workspace root or from a non-member directory under it that defines no
project, the whole workspace is the command's target; when run from a
member (sub)dir, only that member is; a project directory under the root
that is neither a member nor excluded is the hard error of Included
projects, whatever the command. Every command reads `model.toml`
projects and workspaces in v1.

| Commands | From the root or a non-member dir | From a member dir | Selectors |
|---|---|---|---|
| `build`, `sources`, `info`, `publish` | all members (`publish`: in dependency order) | that member | `--workspace` forces all members, `--member <path-or-name>` picks one; KPARs always land in the root `output` |
| `add`, `remove`, `include`, `exclude`, the `info` verbs that write | error: these need a project, so `--member` is required at the root | that member | `--member`; `add --workspace` targeting `[[workspace.inline-usages]]` is a later addition |
| `lock`, `sync` | the whole workspace | the whole workspace: the lockfile is shared, so anything else would be meaningless | none |
| `init`, `new` | create the project and append it to `members` | error until the new project is listed or excluded, as for any unlisted project | none |
| `migrate` | error unless `<path>` names a json project | not applicable: a member is already toml | `<path>` |
| `print-root` | the workspace root | the workspace root | `--member` prints the member's root |
| `env` | per workspace: the single `.sysand` | same | none |
| `clone`, `index`, `auth` | scope-free: `clone` creates a project outside any scope, `index`/`auth` are configuration | same | none |

Two of Cargo's features have no counterpart: `default-members` (`build`
from the root builds all members) and a root package (see Included
projects).


## Extensibility

Arbitrary metadata support is planned, but not for the initial release

## Unknown keys

For future compatibility, unknown keys are allowed, but generate a
warning upon read. This holds for `model.toml` as a whole, `[project]`
and `[workspace]` alike, and for the `urls` tables. A key that belongs
to another metamodel's schema (`root` in an OMG-metamodel project) is
also a warning, one that names the metamodel supporting the key (see
model-toml.md, Metamodel).

The one exception is a usage entry: an unknown key inside an
`[[inline-usages]]` or `[[workspace.<...>inline-usages]]` entry is an
error. A usage entry's kind is decided by which kind key it carries (see
model-toml.md, Usages), so a reader that ignored a kind key it did not
know would read the entry as a different kind, and a reader cannot tell
a kind key it does not know from any other unknown key.

The same hole is why every `model.toml` declares the manifest format
version it was written for (see model-toml.md, Manifest version): a
reader too old for a file refuses it with a message that says so,
instead of misreading it. Together with the metamodel rules the two
signals compose: a reader meeting a newer file sees whichever of the
two its author bumped, and additions gated by the metamodel need no
version bump.

## Non-goals for v1

- Nested workspaces: a workspace root that lies inside another workspace
  errors out, as their interaction is undefined, unless the outer
  workspace `exclude`s it, in which case the two are separate
  workspaces. Detection is limited to the upward walk from cwd (see
  Config): nothing scans for workspace roots below the one in use
- A root package: the workspace root never contains a project (see
  Included projects)
- Nested members (see Included projects)
- Cargo's `default-members` (see Command scopes)
- Several versions of one dependency in one workspace (see Dependency
  resolution)
- Globs in `members`, `exclude` and `sources`
