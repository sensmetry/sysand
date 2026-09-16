# Project definition

KerML 1.0 spec, 10.3 defines the KPAR format. Its intended purpose
is to serve as minimal metadata for exchanging and consuming projects,
and is thus poorly suited to be a project definition during
development.

Problems:
- no clear way to keep precise fields like `meta.checksum` and
  `meta.index` up to date as the project is being developed
- no comments allowed in `.json` files
- underspecified, requiring guesswork to determine e.g.
  how to obtain projects and interpret versions/versionConstraints
- not strict enough, making it difficult to e.g. use package names
  in URLs or filesystems (this is about the syntax of names, not about
  extra keys: unknown keys are allowed with a warning everywhere except
  inside a usage entry, see workspaces.md, Unknown keys). The syntax
  below is what a name may be written as; what reaches a URL or a path
  is its normalized form (see Normalization)
- project does not self-identify in the same way that dependencies
  are specified (name vs IRI)

`model.toml` defines a project, in a `[project]` table. The same file
name is used for workspace configuration, in a `[workspace]` table (see
workspaces.md). Exactly one of the two appears in a file: they never
appear together, as a workspace root must not contain a project, and a
`model.toml` with neither is an error rather than a file that defines
nothing -- which is what a mistyped table name would otherwise become.

A more specific name would be nice, but since the project can use
KerML/SysML2/UAF/whatever, no good choice can be made without implying
that a particular language is used; `model` is neutral in that respect.

## Manifest version

Every `model.toml`, whether it defines a project or a workspace, carries
a top-level integer key stating the manifest format version it was
written for:

```toml
manifest-version = 1

[project]
# ...
```

The key is mandatory and not inheritable: a member's file must be
self-describing before the `[workspace]` table it inherits from is
read. It is read before anything else, so that a file this Sysand
cannot read is refused with the right message rather than with a schema
error:
- absent: an error naming the key
- greater than the highest version this Sysand supports: the error
  "`<file>` is written for manifest-version N; this Sysand supports up
  to M; update Sysand"

Additions to the common schema -- a new key on usage entries, say, such
as an explicit `index` (see Usages) -- bump the version. Additions
gated by `metamodel` (see Metamodel) do not: the metamodel value
already tells a reader which schema applies. `sysand init`, `sysand
new` and `sysand migrate` write the current value. The rationale for
making the key mandatory is under workspaces.md, Unknown keys.

## Main project metadata

Main project metadata is in the `[project]` table of `model.toml`:
- required:
  - `publisher`: same requirements as sysand.com: `^[a-zA-Z0-9](?:[a-zA-Z0-9]|[- ](?=[a-zA-Z0-9])){1,48}[a-zA-Z0-9]$`
  - `name`: same requirements as sysand.com: `^[a-zA-Z0-9](?:[a-zA-Z0-9]|[-. ](?=[a-zA-Z0-9])){1,48}[a-zA-Z0-9]$`
  - `version`: SemVer 2.0
  - `sources`: array of paths to the project's source files, relative to
    the project root (the directory containing `model.toml`). Always
    required for the OMG metamodel (see Metamodel): there is no implicit
    default set of source files, so a project that has none is written
    as `sources = []`. Entries are relative and `/`-separated. On read
    every entry is normalized: a leading `./` and inner `.`/`..`
    components are resolved and repeated slashes are collapsed. The
    normalized form is what the manifest is taken to say: it is what
    the `checksum` and `index` maps of a built KPAR are keyed by, and
    what every command that writes `sources` (`sysand include`, `sysand
    migrate`) writes. An entry that escapes the project root once
    resolved, or resolves to the root itself, is an error, so that every
    listed file has a place in a KPAR. Every entry names a single file:
    an entry with a trailing `/` names a directory and is an error;
    directories and globs are postponed to after v1. Two entries that
    normalize to the same file are a duplicate and rejected. Every entry
    names a KerML (`.kerml`) or SysML (`.sysml`) file, the extension
    compared case-insensitively as `sysand include` compares it today;
    any other entry is an error. Files of other kinds are not listed in
    `sources`: should a need arise, a later version adds a dedicated key
    for them (a `sources_json`, say) rather than widening this one. Whether
    the files exist is checked only for build/publish, as with the
    license file, so reading a manifest never touches them. Unlike the
    keys below, `sources` is per-project only: it cannot appear in
    `[workspace]` and cannot be inherited by a member (see
    workspaces.md), since the file layout of one project says nothing
    about another's
- optional:
  - `license`: SPDX expression, with identifiers validated against a
    pinned SPDX License List version: the one carried by the `spdx`
    crate version that Sysand's own lockfile resolves. Bumping that pin
    is a deliberate change, as it can reject previously accepted
    identifiers. No key names a license file: each identifier in the
    expression maps to `LICENSES/<id>.txt` by REUSE convention, and the
    presence of those files is only checked for build/publish
  - `urls`: table of URLs, currently allowed keys: website, repository,
    documentation. Any other key is an unknown key like any other (warned
    about, ignored, and dropped on build; see workspaces.md, Unknown keys)
  - `description`: string
  - `topics`: array of strings
  - `metamodel`: URL. Besides being copied into the KPAR, it selects the
    schema the rest of the manifest is validated against; see Metamodel
  - `maintainers`: array of strings, each of the form `name <email>`;
    both parts are mandatory. Kept as strings (rather than
    `{name, email}` tables) so that the round-trip to a KPAR's
    `.project.json` `maintainer` array is the identity
  - `stdlib`: string, only for the projects that make up the standard
    library; see Standard library. Not inheritable
- reserved:
  - `root`: belongs to the schema of the Syside metamodel (see
    Metamodel), which is specified after v1. A v1 reader that meets it
    warns that `root` is only supported by the Syside metamodel and then
    applies the recognized metamodel's own required-key check, so `root`
    without `sources` in an OMG-metamodel project still errors on the
    missing `sources`

## Reading and validating

Parsing a manifest never fails on its content: every field of every
manifest -- `model.toml`, and the `.project.json`/`.meta.json` of json
projects and KPARs alike -- is optional at parse time. Whether a key is
required, whether its value has the right shape, and every rule that
spans keys is checked by Sysand's own validation, which reports every
problem it finds in one report, with the position of each. Consequences:
- every error message is Sysand's, so it can name the metamodel (see
  Metamodel) and list all problems at once, instead of stopping at the
  first missing field with a generic parser message
- required-key checks that depend on the metamodel (`sources`) run after
  inheritance has been resolved, since `metamodel` may itself be
  inherited from a workspace group. `manifest-version` (read first) and
  the keys required by every metamodel (`publisher`, `name`, `version`)
  are checked before that
- unknown keys are reported as workspaces.md, Unknown keys describes:
  a warning everywhere except inside a usage entry, where an unknown
  key is an error

## Metamodel

`metamodel` is a URL. Two families are recognized:
- the OMG metamodels, `https://www.omg.org/spec/<spec-kind>/<date>`,
  where `<spec-kind>` is `SysML` or `KerML` and `<date>` is the release
  date, e.g. `https://www.omg.org/spec/SysML/20250201`
- the Syside metamodels, `https://www.sysand.com/spec/<spec-kind>/<date>`,
  mirroring the OMG value they extend: the same `<spec-kind>` and
  `<date>` as the OMG metamodel a project would otherwise use, e.g.
  `https://www.sysand.com/spec/SysML/20250201`

Any other value is a warning, not an error. A project without a
`metamodel` is treated as an OMG-metamodel project, which is also what
a `.meta.json` without the field means.

The metamodel selects the schema of the rest of the manifest. The
common schema -- everything in Main project metadata apart from
`sources` and `root`, plus `inline-usages` -- applies to every project.
On top of it:
- an OMG-metamodel project lists its files in `sources`, and a build
  computes the `index` map of its KPAR from them (see TOML to KPAR).
  This is the schema v1 supports
- a Syside-metamodel project names a single `root` file instead of
  `sources`, and its KPAR carries `root` in `.meta.json` in place of
  `index`, with `checksum` covering every file in the archive. That
  schema is specified after v1; v1 only reserves the key

The rules a v1 reader follows so that a manifest written for a schema
it does not know is reported as such rather than as a broken project:
- (a) when a required key is missing, or an unknown key is present, in
  a project whose `metamodel` is not recognized, the error (for a
  missing key) or the warning (for an unknown key) says the problem may
  be because the project uses a metamodel this version of Sysand does
  not support, and prints the value
- (b) a key that belongs to another metamodel's schema (`root` in an
  OMG-metamodel project) is a warning naming the metamodel that
  supports it, followed by the recognized metamodel's own required-key
  check. An OMG-metamodel project that carries both `sources` and
  `root` therefore gets only the warning and is built from `sources`;
  that is the same cost every other unknown key has, and the warning
  names the fix
- a project whose `metamodel` is a recognized Syside value is validated
  against the OMG schema, the only one v1 knows, and the resulting
  error for the missing `sources` names the Syside metamodel and says
  this version of Sysand does not support its schema

Two checks run over pairs of projects, for every edge of the solved
dependency graph, once before the lockfile is written and again when
the lockfile is validated on `sync` and `build`:
- direction: an OMG-metamodel project must not depend on a
  Syside-metamodel project. This is an error: "`<pub>/<name>` uses the
  OMG metamodel `<url>` and cannot depend on `<pub2>/<name2>`, which uses
  the Syside metamodel `<url2>`; only Syside-metamodel projects may
  depend on Syside-metamodel projects". Because every edge is checked,
  the rule is transitive without further bookkeeping, and a KPAR built
  from a Syside-metamodel project is covered by the metamodel its
  `.meta.json` declares. Inside a workspace the error names both members
  when the offending usage is member-to-member
- release: a `<date>` that differs between a project and one of its
  dependencies is a warning: "`<pub>/<name>` targets metamodel release
  `<d1>` but its dependency `<pub2>/<name2>` targets `<d2>`". Only the
  date is compared, across families and spec kinds, so a Syside
  `SysML/20250201` project depending on an OMG `KerML/20250201` one
  matches. Through the standard library's own `.meta.json` this also
  reports a project whose standard library release differs from its
  own

A json project without a `metamodel` cannot be checked for its release
and is treated as OMG with an unknown date: the direction rule applies,
the release warning does not.

## Usages

`inline-usages` contains the projects this project uses. It is an array
of tables, normally written as `[[inline-usages]]`. A second, more
ergonomic shape will be added after v1; hence `inline`.

Terminology: what a manifest declares is a *usage* (`dependency` clashes
with SysML). The relationships between projects and the machinery that
resolves them keep the usual names: dependency graph, circular
dependencies, dependency resolution, dependency solver.

Each element is either a usage declared here, or an opt-in to a usage
declared by a workspace: an entry carrying a `workspace` marker gives only
the identity and is replaced by the matching workspace group entry before
any of the below applies, so it has no source kind of its own (see
workspaces.md, Common usages; such an entry cannot be read outside a
workspace).

A declared element must be a table containing one of these sets of
properties (in the form of
`source-kind: field1, field2 + optional-field1, optional-field2`, where
the fields name the TOML keys). Which kind an entry declares is decided by
its kind key -- `dir`, `kpar-path`, `git`, `url` -- of which at most one
may be present; an index usage is the entry that carries none. A version
requirement, wherever a kind takes one, is always `version`.

Every usage carries `publisher` and `name`: that pair is the identity a
usage is resolved by (see Normalization), and it is what the lockfile
records, as `pkg:sysand/<publisher>/<name>`. The target must declare the
same publisher (compared after normalization). A target that declares
none -- possible only for a json project or a KPAR, since `[project]`
requires a publisher -- is an error when the usage resolves: "usage
`<name>` targets `<location>`, whose `.project.json` has no `publisher`;
add one to the target (or migrate it to `model.toml`, where `publisher`
is required)". The one exception is the standard library: a
publisher-less KPAR found at an OMG standard-library IRI counts as
publisher `omg` (see Standard library). A standard-library KPAR built
locally (in `output/`) is not at such an IRI and is not recognized, so
as a `kpar-path` target it is the error above like any other
publisher-less KPAR; the standard library's own projects reference each
other by `dir`, which reads the sibling's `model.toml`. Nothing in the
manifest can tell whether a
target declares a publisher, so the check happens at resolution, and a
target gaining or losing its publisher invalidates the usages that refer
to it.

- index: publisher, name, version constraint. Index usages are implicit
  in v1: the entry names no index, and resolution looks the identity up
  in the configured indexes (`sysand.toml` `[[index]]`, `--index`,
  `--default-index`). An optional `index = "<name>"` key may be added
  later; it would be an attribute of the index kind, not a kind key, so
  kind detection is unaffected, but because a v1 reader would warn about
  it and silently resolve against its own indexes, adding it bumps
  `manifest-version` (see Manifest version). An index name cannot
  survive into a KPAR, whose usages are PURLs (see TOML to KPAR); the
  lockfile records the index URL, as it does today
- source path: dir, publisher, name. `dir` is relative to the directory
  of the `model.toml` the entry is written in: the project root for a
  project's own entries, the workspace root for a workspace group's
  entries (see workspaces.md, Common usages)
- kpar path: kpar-path, publisher, name. Relative as for `dir`

  A `dir` or `kpar-path` is taken exactly as written, in the one form
  Sysand writes: `/`-separated, with `..` only as leading components
  (`../../libs/a`), and no `.` component, no repeated or trailing slash
  and no backslash. Any other spelling (`./a`, `a/`, `libs/../a`) is an
  error rather than normalized. Unlike a `sources` entry, such a path may
  leave the directory it is relative to, which is what the leading `..`
  are for
- git: git, publisher, name + rev/tag/branch
  (`git` names the repo, either by a remote URL or by a path to a
  local clone; nothing points inside the repo)
  Postponed to after v1: not declarable in v1
- remote-kpar: url, publisher, name -- this is used for
  standard-compliant KPARs; version constraint not supported
- `resource` (the KerML usage type carried by json projects, see
  `InterchangeProjectInfo`) is legacy and is not supported directly:
  it cannot be declared in `model.toml`. It only appears on the KPAR
  side, as what the kinds above are turned into when building one
- Remote source is not included for simplicity; very few legitimate uses exist;
  supporting this is slow, and with now two supported metadata formats,
  both need to be checked, and this increases the latency; file system
  source projects need the same double file checking, but local fs latency
  is much smaller. Should it ever be added, its key is `dir-url`, the
  remote counterpart of `dir`

An unknown key inside a usage entry is an error, not a warning, because
a reader cannot tell a kind key it does not know from any other unknown
key (see workspaces.md, Unknown keys).

### Circular dependencies

Circular dependencies are an error. The check runs over the dependency
graph whenever it is built -- when a workspace is read, and when
resolving or building -- and the error lists the projects on the cycle.
The one exception is the standard library: a cycle every project of
which carries the `stdlib` key (see Standard library) is allowed, since
the Kernel libraries of the standard library depend on each other. A
cycle that includes one project without the key is an error like any
other. How cyclic standard-library projects build is under
workspaces.md, Build output.

### Semver dependencies

Every project in a project's dependency closure -- its usages, their
usages, and so on -- must have a `version` that parses as SemVer 2.0,
so that constraints such as `^1.0.0` and `>=1.2` can be compared and
matched against it. `[project]` guarantees this for `model.toml`
projects; a json project or a KPAR may carry any string. Since the
closure cannot be established from a manifest alone, the check happens
at resolution: a usage that resolves to a project whose version is not
semver is an error that names the usage chain from the resolving project
to the offending one and quotes the version, e.g. "`acme/app` -> `acme/lib`
-> `legacy/util`: `legacy/util` declares version `2023-11`, which is not
semver". Lockfile validation does not repeat the check: a lockfile
records no version constraints, and one written by resolution never
contains a non-semver version for a `model.toml` project, so a
non-semver version met in a lockfile stays the warning it is today.
Kinds that carry no
version constraint (remote-kpar) and json targets are therefore not
forbidden up front; they fail only if what they resolve to is not semver.

### Normalization

A project's *identity* is its `(publisher, name)` pair. The normalized
form of a publisher or name is its ASCII-lowercased spelling with spaces
replaced by hyphens; dots in a name are left as they are. Two identities
are equal when their publishers and names are equal after normalization,
so `Acme Corp` and `acme-corp` denote one project. Where this document
says two spellings are compared *literally*, it is the spellings that
are compared, not the identities.

All projects are resolved by their *normalized* identity, regardless
of how the usage is declared. This is to ensure consistent resolution
for existing PURL usages in json projects. Resolution is the only place
that normalizes -- index lookups, the lockfile identifier
`pkg:sysand/<publisher>/<name>` and the PURLs built when packaging a
KPAR included. Every other comparison is literal string equality,
workspace group matching (see workspaces.md, Common usages) among them,
so an opt-in entry has to spell publisher and name exactly as the group
entry does.

The exception is duplicate detection within one array of usages: two
entries whose identities normalize to the same value are the same project
declared twice and are rejected, whether or not the two entries agree on
kind or constraint. The arrays are counted separately, since a workspace
config holds several (see workspaces.md, Common usages).

Normalization never rewrites a manifest: `[project]` and every usage keep
the spelling as written. A usage that `sysand migrate` derives from a
PURL is the one case where the spelling is already normalized, since a
valid `.project.json` only carries normalized PURLs (see JSON to TOML).

## Standard library

The standard library is published under the publisher `OMG`, which
normalizes to `omg`. A project declares its standard-library usages
like any other index usages:

```toml
[[inline-usages]]
publisher = "OMG"
name = "Kernel Semantic Library"
version = "^1.0.0"
```

Every release of the standard library gets a distinct version, so the
version constraint is how a project picks a release; a version
constraint on a standard-library usage is therefore allowed, in
manifests and at publish time alike. Inside a workspace, projects that
need different releases put their standard-library usages in different
groups (see workspaces.md, Common usages).

### Provided by the tooling

A usage is a standard-library usage iff its publisher normalizes to
`omg`; a json project's usage of one of the OMG standard-library IRIs
(`https://www.omg.org/spec/<KerML|SysML>/<date>/<stem>.kpar`) is the
same project under its other spelling, and a publisher-less KPAR found
at such an IRI counts as publisher `omg` (only there: see Usages for a
locally built one). Sysand serves two kinds of
clients with that identity, as it does today:
- by default, the standard library is *provided by the tooling*: Sysand
  embeds the metadata of the current release, `sysand lock` records the
  standard-library entries with an empty source list, `sysand sync` does
  not install them, and a lockfile that pins a release the tooling does
  not provide is an error on `sync`
- with `--include-std`, the standard library is resolved and installed
  like any other dependency, from an index included

### Developing the standard library

The standard library itself is developed as a `model.toml` workspace.
Its ten projects have publisher `OMG`, one group per language carrying
the `metamodel` of that language's release, and each `[project]` table
carries the key `stdlib`:

```toml
[project]
publisher = "OMG"
name = "Kernel Semantic Library"
version = "1.0.0"
stdlib = "Semantic-Library"
metamodel.workspace = "kerml"
sources = ["Base.kerml", "Links.kerml", "Occurrences.kerml", "Objects.kerml"]
```

`stdlib` is a string: the last segment of the IRI at which the
library's KPAR is published (`https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar`),
validated as a single path segment (`[A-Za-z0-9][A-Za-z0-9-]*`) so it
cannot carry `/` or `..` into an IRI. It is not inheritable, being a
per-project identifier, and an absent key means the project is not part
of the standard library. It is refused unless the project's publisher
normalizes to `omg` and its metamodel, declared or inherited, is a
recognized OMG one (the IRI needs its release date), so it cannot be
used to give other projects the standard library's form.

What the key changes when a KPAR is built (see TOML to KPAR for the
normal rules):
- a usage whose target carries `stdlib` is written as
  `resource = "<target's metamodel>/<target's stdlib>.kpar"` -- the
  metamodel supplies the prefix and release date -- with its version
  constraint, instead of as a PURL
- `publisher` is omitted from the `.project.json` of a project that
  carries the key

Nothing else is special: the archive is named as for any other project,
and `created` is the time of the build.

The requirement behind the key is that the KPARs built this way are
*equivalent* to the ones OMG publishes from the same sources.
Equivalent means: both `.project.json` files parse to equal JSON values
with key order ignored and `versionConstraint` compared as parsed
ranges (`1.0.0` and `^1.0.0` are the same range); both `.meta.json`
files parse to equal JSON values with key order ignored, `created`
ignored, and a `checksum` map allowed in the new file where the old one
lacks it; and the source files are byte-identical under the same
relative paths. This definition is executable, and a test that builds
the standard-library workspace and compares each produced pair against
the release metadata Sysand embeds is the acceptance criterion; the
test needs the standard library's sources checked out next to that
metadata.

## TOML to KPAR

A KPAR is spec-compliant: it carries `.project.json` and `.meta.json`,
never `model.toml`. `sysand build` writes it to `output/` under the
project root -- the workspace root for a member (see workspaces.md,
Build output) -- as `<name>-<version>.kpar`, with every character of the
name that is not alphanumeric replaced by `_`.

- `.project.json`:
  - `publisher` goes into `project.publisher`, unless the project
    carries `stdlib` (see Standard library)
  - the remaining `[project]` keys map onto their counterparts:
    `name`, `version`, `description`, `license` as-is, `topics` into
    `topic`, `maintainers` into `maintainer`, `urls.website` into
    `website`. `urls.repository` and `urls.documentation` have no
    counterpart in `.project.json` and are dropped
- `.meta.json`:
  - `metamodel` is copied into the field of that name
  - `created` is the time of the build (so two builds of identical
    sources are not byte-identical), and `includesDerived`/
    `includesImplied` are optional and omitted
  - `sources` is what gets packaged as the project's model, and is also
    the input for the two maps that have no counterpart in `model.toml`,
    both computed at build time from the file contents and keyed by the
    normalized `sources` entries:
    - `checksum`: an entry per source file, keyed by its path, holding
      the file's hash (SHA256, as `sysand include` already produces)
    - `index`: the top level symbols extracted from each source file,
      each mapped to that file's path; the language is taken from the
      file extension (KerML or SysML), which `sources` guarantees to be
      one of the two
    This is why `sources` itself is not written to the KPAR: the paths
    it lists are recoverable from these two maps, which is exactly the
    union the reverse direction takes (see JSON to TOML)
  - a build packages conventional files besides the sources: `README.md`,
    `CHANGELOG.md`, and the `LICENSES/<id>.txt` named by the license
    expression. These are found by path and cannot be listed in
    `sources`, which admits only KerML and SysML files (see Main project
    metadata); they get neither a `checksum` nor an `index` entry --
    which is what keeps the union above equal to `sources`
- usages are transformed from the `[[inline-usages]]` array, after any
  workspace opt-ins have been replaced by their group entries:
  - index: turn into a PURL (`pkg:sysand/<publisher>/<name>`) with the
    version constraint as declared
  - source path, kpar path: turn into a PURL constrained to
    `^<version>`, where `<version>` is the target project's version at
    build time. There is no fallback: a publisher/name pair that cannot
    be encoded into a PURL is a build error, and so is a target whose
    version is not semver -- possible for a json target, since
    `model.toml` constrains only its own version -- since then there is
    no `^<version>` to emit. `sysand build --allow-path-usage`, the
    existing flag whose meaning is "I accept a KPAR that only resolves
    on this machine", is the opt-out: with it, local usages are emitted
    as the typed usages `Directory` (source path) and `KparPath` (kpar
    path), which preserve the path as written and cannot be expressed
    via `resource`, with the existing warning that the result is not
    portable. A typed usage carries `publisher` and `name` exactly as
    the manifest spells them, without normalization, because Sysand
    requires them to match the values found at the target
  - an index, source path or kpar path usage whose target carries
    `stdlib`: the OMG IRI instead of the PURL (see Standard library).
    Under `--allow-path-usage` a local usage stays a typed path usage,
    stdlib target or not; a remote-kpar usage is always its URL
  - git: cannot occur in v1, as git usages are not declarable there.
    When introduced: not supported, because the project may not be at
    repo root; proper support requires typed git usage in json project
  - remote-kpar: use the URL as-is

`build` never resolves usages, so building needs neither network nor
index; the target versions it needs come from the targets' manifests
(members and local paths), never from built artifacts.

## JSON to TOML

json = old project format, `.project.json` + `.meta.json`
toml = new project format

A json project is supported without warnings, and may be the target of
a usage, but cannot be a workspace member (see workspaces.md, Member
requirements). It is migrated to
toml with a dedicated command, `sysand migrate [<path>]`, defaulting to
the project of the current directory:
- one project per run. A json project cannot be a member, so there is
  no workspace-wide form; from a workspace root the command errors
  unless `<path>` names the project
- the project's git tree must be clean -- `git status --porcelain`
  over the project root empty, counting staged, unstaged and untracked
  files and ignoring ignored ones -- checked before anything is
  written; `--allow-dirty` bypasses the check. Outside any git work
  tree the command also refuses; `--allow-no-vcs` bypasses that. Both
  because the command rewrites files the user cannot regenerate
- `--publisher <publisher>` supplies the one value the json side
  usually lacks (see non-spec fields below); when the command is
  interactive it may prompt for it instead. Without one the command
  errors before writing, since `[project]` requires it
- it writes `model.toml` with the current `manifest-version` in the
  generated-file layout (see Editing the manifest), then deletes the
  `.project.json` and `.meta.json` it replaced, so that the project is
  left in a state every other command accepts (see Mixed manifests).
  Version control keeps the old files' history
- best-effort: any value that does not fit the toml shape is an error,
  and the command writes nothing

Field by field:
- `.project.json`:
  - `name`: auto for compatible names, error out otherwise
  - `description`: use as-is
  - `version`: auto for semver, error out otherwise
  - `license`: auto for spdx, error out otherwise
  - `maintainer`: auto for elements of the form `name <email>`, which
    are used as-is; error out otherwise (e.g. a bare name, a bare
    `<email>`, or `some <some-email> strings`)
  - `website`: put as-is into `urls.website`
  - `topic`: put as-is into `topics`
  - `usage`:
    - `resource`:
      - if `http(s):`, use as-is for URL, discard `versionConstraint`
        if present with a warning. The publisher and name come from the
        target, which is fetched; a target without a publisher is an
        error (see Usages)
      - if `ssh`/`git`/`git+`, error out: git usages are not
        declarable in v1 (see Usages)
      - if `file:`, translate into kpar/source path; the publisher and
        name come from the target, and a target without a publisher is
        an error
      - if Sysand PURL, translate into an index usage. A valid
        `.project.json` only carries normalized PURLs, so the migrated
        usage spells publisher and name in normalized form; a workspace
        group entry it is later made to opt into has to use that
        spelling (see Normalization). `versionConstraint` is used
        as-is. Index usages require a constraint (see Usages), so a PURL
        that carries none is resolved at migration time and pinned to
        `^<version>`, where `<version>` is the version that resolution
        found; migration of such a usage therefore needs the index to
        be reachable
      - (for all of these, may just use the resolver + `sources()`)
      - error out if any other type is encountered or if the missing
        data could not be populated due to any error
    - source path (`Directory`), kpar path (`KparPath`): the path,
      rewritten into the form a `dir` or `kpar-path` requires (see
      Usages) -- a leading `./`, a `.` component and a trailing slash
      dropped, a `..` resolved against what precedes it unless that is a
      `..` too. A path that is left empty names the project itself, and
      one with a repeated slash is refused, as before; both are errors
- `.meta.json`:
  - `sources`: the union of the paths in `index` (its values, the
    symbol names being dropped) and in `checksum` (its keys, the
    checksums themselves being dropped), each normalized as `[project]`
    normalizes `sources` entries and then deduplicated. Both maps are
    optional in a json project, so the union may be empty, which
    becomes `sources = []`. The spec requires no relationship between
    the two maps, hence the union rather than either one alone. A path
    that `[project]` would not accept -- an absolute one, or one
    escaping the project root -- errors out, as for any other json
    value that does not fit the toml shape. So does a `checksum` key
    naming a file that is neither `.kerml` nor `.sysml` (possible today
    through `sysand include --no-index-symbols`), since `sources` admits
    only those: the error lists the entries, and the fix is `sysand
    exclude` on the json project before migrating
  - discarded as irrelevant/impossible to keep up-to-date with a warning:
    - `created`
    - `checksum` (apart from its paths, see `sources` above)
    - `index` (apart from its paths, see `sources` above)
  - `metamodel`: use as-is
  - `includesDerived`: discard, no purpose in development; a built
    KPAR omits it (see TOML to KPAR)
  - `includesImplied`: discard, likewise
- non-spec fields:
  - `publisher`: same treatment as `name` when present. Spec-compliant
    projects have no such field, so it is usually absent; migration then
    takes the publisher from `--publisher` (or a prompt) and proceeds as
    if it had been declared, validating it like any other publisher.
    Migration cannot complete without one, as `[project]` requires it
- unknown fields: print a warning about unknown fields and discard

## Mixed manifests

Which manifests a directory holds decides what it is:
- current project has only json config: supported, no warnings, but it
  cannot be a workspace member; `sysand migrate` converts it
- current project has only toml config: supported, no warnings
- current project has toml and json configs: error out that it's unclear
  if the project is meant to be a non-zipped KPAR, or dev project;
  same whether or not it belongs to a workspace. `sysand migrate` is the
  one command that reads both, and it leaves only the toml one behind
- the cases below do not depend on whether the referring project is toml
  or json: what matters is the usage kind and what is found at the target
- source usage target has only json config: supported, no warnings
- source usage target has only toml config: supported, no warnings
- source usage target has json and toml configs: error out that it's
  unclear if the project is meant to be a non-zipped KPAR, or dev project
- kpar usage target has only json config: supported, no warnings
- kpar usage target has only toml config: not supported, error out
- kpar usage target has json and toml configs: error out same as for source

TLDR:
- KPARs will continue to exclusively use `.project.json`/`.meta.json`
  (along with typed usages, which remain available behind
  `--allow-path-usage`, as these can't be expressed via `resource`)
- if toml config present in a KPAR, error out
- source projects can be either toml or json, but if both present errors out
- only toml projects support workspaces: a `members` entry that resolves
  to a json project errors out, since members must use the `model.toml`
  project definition (see workspaces.md)

## Editing the manifest

Every command that writes a `model.toml` -- `sysand add`, `sysand
remove`, `sysand include`, `sysand exclude`, the `set`/`clear`/`add`/
`remove` verbs of `sysand info`, and the rewrites that turn a value into
an inheritance marker -- edits the existing document in place, preserving
comments, key order and formatting; comments are the reason a TOML
manifest exists, so no writer may drop them. Freshly generated files
(`sysand init`, `sysand new`, `sysand migrate`) are rendered through the
same document type in a fixed layout, without comments: `manifest-version`
first, then `[project]` with its keys in the order Main project metadata
documents them -- except `urls`, written as a `[project.urls]` table
after the scalar keys, as TOML requires -- with `sources` as a
multi-line array with one file per line and a trailing comma (an empty
one as `sources = []`), then `[[inline-usages]]` tables in manifest
order; a workspace file has `manifest-version` and then `[workspace]`.
One code path serves generated and edited files, and the fixed order
diffs cleanly.

Every command accepts `model.toml` projects in v1. Two adjustments
follow from `sources` being the only file list:
- `sysand include <paths>` appends each path to `sources`, normalized
  as on read, rejecting a duplicate; `sysand exclude <paths>` removes it.
  Neither extracts symbols or hashes files, as both maps are computed at
  build; `--compute-checksum` and `--no-index-symbols` are rejected on a
  toml project with the message that the field is computed when the
  KPAR is built. A path that does not exist gets a warning (existence
  is only checked at build and publish, but including a missing file is
  almost always a typo); paths that are not `.kerml`/`.sysml` are
  allowed and get only a `checksum` entry at build
- the `sysand info` verbs on the fields a build computes -- `index`,
  `checksum`, `created`, `includesDerived`, `includesImplied` -- error
  on a toml project with the same message; `info index get` may compute
  the map on the fly

## Non-goals for v1

Change the packaging format. The KPAR format as defined in the spec
remains the only supported packaging format for now.


## External constraints

- file names etc. must be vendor-neutral
- KPAR support must be retained in archived and extracted forms
- the standard library must be developable as a `model.toml` workspace,
  producing KPARs equivalent to the published ones (see Standard
  library)
- must accommodate Pilot's use cases:
  - support workspace inheritance groups, needed for:
    - kerml/sysml std versions differ, but are the same for the same lang
    - same with metamodels
