# The Syside metamodel schema

model-toml.md, Metamodel, says the metamodel decides "the schema of the
rest of the manifest": a common schema applies to every project, and on
top of it each family has its own. That document specifies the common
schema and the OMG family's. This one specifies the Syside family's --
what a `model.toml` whose `metamodel` is a Syside value carries beyond
the common schema, how it is read, what its KPAR looks like, and how it
behaves as a dependency.

The Syside family is the two values `https://www.sysand.com/spec/KerML/<date>`
and `https://www.sysand.com/spec/SysML/<date>` (model-toml.md,
Metamodel). Everything model-toml.md says that is not about `sources`'
companion key `root` or about the shape of the KPAR applies to a Syside
project unchanged: names, versions, licences, usages, the standard
library, normalization, editing, the release and direction checks.
This document does not restate them.

Nothing here bumps `manifest-version`. model-toml.md, Manifest version:
"Additions gated by `metamodel` [...] do not bump anything: the
metamodel value already tells a reader which schema applies." A reader that does
not know this schema meets a manifest whose `metamodel` it recognizes
and whose `root` it can name; what it says is under Reading and
validating.

## `root`

A Syside-metamodel project names one of its source files as its
**root**:

```toml
manifest-version = "1.0.0"

[project]
publisher = "acme"
name = "widgets"
version = "1.0.0"
sources = ["Widgets.sysml", "Parts.sysml", "Ports.sysml"]
metamodel = "https://www.sysand.com/spec/SysML/20250201"
root = "Widgets.sysml"
```

- `root` is **required** of every Syside-metamodel project, as `sources`
  is required of every project (see `sources` below). A Syside project
  without it is an error naming the key and the family (see Messages
  under Reading and validating).
- `root` **names one of the `sources` entries**, compared as written --
  the entries themselves are compared as written (model-toml.md, `sources`:
  "Two equal entries are a duplicate and rejected", equal, not
  equivalent), and so is this. A `root` that is not an entry is an
  error. Because `root` must be an entry, a project with
  `sources = []` cannot have a valid `root`, and so a Syside project
  always has at least one source file.
- `root`'s **syntax is exactly a `sources` entry's**: relative,
  `/`-separated, no `.` or `..` component, no repeated or leading slash,
  not a directory, a `.kerml` or `.sysml` file (model-toml.md, `sources`).
  There is one path rule in a manifest, not two. A `root` that breaks a
  rule is an error worded for the key; a `root` that is not a
  string is the type error every other key gets. Whether the file
  exists is checked only for build/publish, as for every `sources`
  entry.
- Which of `.kerml` and `.sysml` the `root` may be is the metamodel's
  spec kind's decision, exactly as for the entry it is (model-toml.md,
  Metamodel, "Second"). Because `root` is an entry, a `root` the
  metamodel does not admit is reported once, as the entry; there is no
  second complaint about the same file.
- `root` is **not inheritable**. It is the one thing about a project's
  files that names a file, and "the file layout of one project says
  nothing about another's" (workspaces.md, Common metadata). A marker
  on it is an error **for every family**, raised when the table
  is read: whether a key may be inherited does not depend on the
  metamodel. The `metamodel` that decides whether `root` is required
  _is_ inheritable, which is why every _other_ judgement of `root` waits
  for inheritance (see Reading and validating).

### What `root` is for

`root` is the project's **entry point** for a consumer. Two consumers
are specified, both on the Oxide side:

- **Name resolution.** A dependency is reached as `publisher::name`,
  and that name resolves to the _body_ of `root` -- not to a wrapper
  around it -- so that a symbol `root` declares at its top level is
  `publisher::name::symbol`, two segments, rather than
  `publisher::name::topLevelPackage::symbol`, three. A project that
  wants several independent top-level packages writes a facade `root`
  that re-exports them; that is a real constraint on how a Syside
  project is laid out, and it is Oxide's, recorded here so that the key
  is not mistaken for decoration.
- **Deferral.** A consumer that loads dependencies lazily parses `root`
  first, one level deep, to learn what the dependency exports before
  deciding whether to load the rest. `root` is the file it opens.

Neither use asks Sysand to know which files `root` reaches. Sysand
packages `sources`, checks that `root` is one of them, and carries the
value into the KPAR; the consumer does the rest. **Specification
commitment, deferred check:** every `sources` entry _should_ be reachable
from `root` through imports -- a file no import reaches is dead weight in
the archive and invisible to a consumer that starts at `root`. Sysand
cannot compute reachability today (its symbol extractor is a lexer
that does not follow imports), so this is stated as the intent and will
become a **warning** when the check exists. It is not an error now and
will not become one without a `manifest-version` discussion, because a
build that succeeds today must not start failing.

## `sources`

Unchanged from model-toml.md. `sources` is the project's file list for
both families: the files that are packaged, checksummed and, for the
OMG family, indexed. It is required, per-project, taken as written,
edited by `sysand include` and `sysand exclude` exactly as for an OMG
project. A Syside project without it is an error that says where
`root` fits.

`root` is _in addition to_ `sources`, not instead of it. The reason:
what the consumers need is the `index` _map_ collapsed into one entry
point, not the file list removed; a build whose contents
depend on what happens to be on disk is what `sources` exists to
prevent; and reachability, the only rule under which `root` alone could
define the file set, is a check Sysand cannot make.

## Reading and validating

model-toml.md, Reading and validating, holds: every key is optional at
parse time, every rule is Sysand's, and "everything that depends on
[`metamodel`] ... [is] checked after inheritance has been resolved". For
this schema, everything about `root` depends on the metamodel:

- **Under a Syside family** -- declared, or inherited from a group --
  `root` is judged: its type, its shape, its presence and its
  membership of `sources`; `sources` is required. All of it in one
  report with everything else that is wrong, and a `root` that was
  written and refused is not _also_ reported missing.
- **Under the OMG family**, `root` is a key that "belongs to another
  metamodel's schema" (model-toml.md, Metamodel, first rule): a warning
  naming the family that supports it, and **nothing else, whatever the
  value** (a marker excepted, above). The project is built from
  `sources`; `root` is neither read nor judged, so `root = true` costs
  the same one warning as a path does.
- **Under no family** -- the `metamodel` is missing, or a marker on it
  did not resolve -- nothing is said about `root` (a marker excepted,
  again): the missing metamodel is already in the report, and a
  metamodel that says nothing about the schema decides nothing about
  the key. This is where the `model.toml` reader and the `.meta.json`
  reader differ, and why: `metamodel` is mandatory in a `model.toml`, so
  its absence is already an error, whereas an absent `metamodel` in a
  `.meta.json` _means_ the OMG family (model-toml.md, Metamodel), so the
  json reader warns about `root` there.

A reader that recognizes the Syside values but predates this schema
does not refuse a Syside project _with_ `sources` -- it builds it as an
OMG project, `root` warned about and ignored -- and refuses one
_without_ `sources` with a message naming the Syside metamodel as
unsupported. Both are the behaviours model-toml.md promised for a
schema a reader does not know; the second is the missing-`sources`
error in a reader that knows this one.

A member may inherit a Syside metamodel from a group and carry its own
`root`:

```toml
manifest-version = "1.0.0"

[project]
name = "ports"
publisher.workspace = "syside"
version.workspace = "syside"
sources = ["Ports.sysml"]
metamodel.workspace = "syside"
root = "Ports.sysml"
```

(all three markers name one group, `syside`, which supplies the three
values; `root` and `sources` are the member's own)

And an OMG project carrying `root` is a valid OMG project with one
warning:

```toml
manifest-version = "1.0.0"

[project]
publisher = "acme"
name = "legacy"
version = "1.0.0"
sources = ["Legacy.sysml"]
metamodel = "https://www.omg.org/spec/SysML/20250201"
root = "Legacy.sysml"
```

### Messages

Messages follow the house rules: lowercase, user input in backticks,
the fix named where there is one.

| row | situation | text |
|---|---|---|
| P2-01 | Syside project, `root` absent | missing key `root` in `[project]`; the Syside metamodel requires it |
| P2-02 | `root` breaks a `sources` entry rule | one of six: `root` `<path>` is absolute; `root` is relative to the project root, like a `sources` entry · `root` `<path>` contains a backslash `\`; it is `/`-separated, so that a manifest reads the same way on every operating system · `root` `<path>` contains a NULL character · `root` `<path>` is not in the form a `sources` entry takes: relative, `/`-separated, with no `.` or `..` component and no repeated or leading slash · `root` `<path>` names a directory; `root` names a single model file · `root` `<path>` is not a KerML (`.kerml`) or SysML (`.sysml`) file |
| P2-02 | `root` is not a string | `root` in `[project]` must be a string, but it is `<type>` |
| P2-03 | `root`'s extension is not admitted by the metamodel | *subsumed*: `root` is a `sources` entry, and the entry's own complaint (appendix A2 row 47b) is the one message |
| P2-04 | `root` is not a `sources` entry | `root` is `<path>`, which is not listed in `sources` |
| P2-05 | `root` in an OMG-metamodel project | `root` in `[project]` is only supported by the Syside metamodel and is ignored here *(warning)* |
| P2-06 | `root.workspace = true` | `root` in `[project]` cannot be inherited from the workspace; it is a per-project entry point |
| P2-23 | Syside project, `sources` absent | missing key `sources` in `[project]`; the Syside metamodel requires it (`root` names one of its entries) |

The multi-line texts are shown with their line breaks as `;`, as a
parenthesis, or as a space after a comma; the exact bytes are the
tests'.

## The Syside KPAR

*This section specifies what `sysand build` and `sysand publish` do
for a Syside project. Where the code stands: `build` writes the shape
below -- `root`, `exports`, no `index` -- and every reader accepts it;
`build --keep-index` is refused on such a project. Not yet: `checksum`
still covers the source files only, `publish` still runs the OMG
rules over a Syside archive (it warns about the empty `index` it does
not have and checks no symbols), `include --no-index-symbols` is not
refused, and format detection of the extracted form is open. Those are
`plan-phase2.md` WP-A2b's, and this preamble goes when they land.*

A KPAR built from a Syside-metamodel project carries `.project.json` and
`.meta.json` as any KPAR does (model-toml.md, TOML to KPAR), and never
`model.toml`. `.project.json` is unchanged. `.meta.json` differs from an
OMG KPAR's in three ways:

- **`root`** is carried, as written in the manifest -- the same path
  that keys its `checksum` entry.
- **`exports`** replaces `index`: a flat array of the top-level symbols
  declared by every model file in the archive, computed at build time
  by the same extractor that fills an OMG KPAR's `index` map, in
  `sources` order, each name once at its first appearance. It is an array of names, not a map of
  name to file: the map is what the Oxide asks (`constraints.md` C1)
  wanted collapsed, and the file for every symbol is `root`, by
  construction, for a consumer resolving through it. Sysand needs the
  names for one thing: the cross-project symbol-collision check the
  lockfile runs, whose only input is what a project exports and which
  would otherwise stop covering this family (see Dependencies).
- **`index` is absent**, not empty. An empty map would read, to a reader
  of the OMG shape, as a project exporting nothing; absence trips that
  reader's missing-key path, whose message names the metamodel.
  A Syside `.meta.json` that carries `index` anyway is a **warning and
  the map is ignored**, on the same rule as a foreign key in a
  `model.toml` -- everywhere it is read, `publish` included, so that
  there is one rule for the key and not a reader that warns and a
  publisher that refuses. A Syside `.meta.json` with `index` and
  neither `root` nor `exports` -- the OMG shape under a Syside
  metamodel -- gets that warning _and_ both missing-key errors: the
  `index` does not stand in for the keys.

The `.meta.json` reader judges `root` as the `model.toml` reader does:
present, in the form a `sources` entry takes (the `model.toml`
reader's texts, with the file as the location); and `exports` present.
Whether the file `root` names is in the archive is `publish`'s
question, as the existence of every other file is. One difference from
the `model.toml` reader: a `.meta.json` is parsed into typed fields
before Sysand judges it, so a `root` that is not a string is the
parser's error naming the file, as it is for every other key of the
document -- under a Syside _or_ an OMG metamodel. The `model.toml`
reader keeps the value unjudged under OMG because it may be inherited
into a family later; a `.meta.json` has no inheritance, and that is
the one place the json side is stricter than the toml side.

And in one way that is not a difference in shape:

- **`checksum` covers every file in the archive** except the two json
  manifests themselves (`.meta.json` cannot hash itself, and
  `.project.json` is read before `.meta.json` is), where an OMG KPAR
  checksums its source files only. `LICENSES/<id>.txt`, `README.md` and
  `CHANGELOG.md` are therefore checksummed in a Syside KPAR. The archive
  is verifiable whole; a consumer that trusts `checksum` trusts every
  byte it will read.

The archive contains no `model.toml` (a `model.toml` at a KPAR root is
already an error, `LocalKParError::TomlInKpar`); its model files are
exactly the manifest's `sources`, so `root` is among them.

A Syside KPAR is **not a KerML-specification KPAR**: KerML 1.0, 10.3
mandates `index`. That is accepted: only
Syside-metamodel projects may depend on it (model-toml.md, Metamodel,
direction rule), and a consumer of the OMG shape that meets one is told
what it is by the missing-`root` error rather than by a parser.

The **extracted form follows the archive**: a directory holding a
`.project.json` and a `.meta.json` with `root` is a json project of the
Syside family, readable and usable as a `kpar-path` or `dir` target.
How format detection classifies such a directory beside a
`model.toml` is **open**. For a directory, "the archive" in the rules
above is the directory: `checksum` covers the files it lists, and
presence is checked as for an archive.

### What a reader of the OMG shape says

A reader that predates this schema fails on a Syside `.meta.json` with
a missing-`index` error: Sysand 0.2.x inside its parser, with serde's
"missing field `index`"; the reader that made every field optional at
parse with its own "missing key `index` in `<file>`" and no metamodel
named, because the suffix naming the metamodel is appended only when
the value fails to parse. A reader that knows this schema names the
Syside metamodel (P2-11), as the `model.toml` reader names it. Every
consumer that embeds `sysand-core` and reads `.meta.json` must ship a
core that knows this shape before any Syside KPAR is published.

| row | situation | text |
|---|---|---|
| P2-07 | `build --keep-index`, or `include --no-index-symbols`, on a Syside project | `<flag>` applies to projects with a computed index; a Syside project's KPAR carries `root` and `exports` instead |
| P2-08 | publish: `root` names a file that is not in the archive | the archive does not contain `root` `<path>` |
| P2-09 | publish: `checksum` misses a file | `checksum` does not cover `<path>`; a Syside KPAR checksums every file in the archive but `.project.json` and `.meta.json` |
| P2-10 | a Syside `.meta.json` carrying `index` | `index` in `<file>` is not used by the Syside metamodel and is ignored; a Syside KPAR carries `root` and `exports` *(warning, wherever the file is read)* |
| P2-11 | `.meta.json` with no `root` under a Syside metamodel; with no `index` under any other | missing key `root` in `<file>`; the Syside metamodel requires it *-- and under any other family, the existing* missing key `index` in `<file>` *unchanged* |
| P2-22 | `.meta.json` carrying `root` or `exports` under an OMG or absent metamodel | `<key>` in `<file>` is only supported by the Syside metamodel and is ignored here *(warning, one per key; P2-05's text with the file as the location)* |
| P2-24 | `.meta.json` with no `exports` under a Syside metamodel | missing key `exports` in `<file>`; the Syside metamodel requires it |
| P2-25 | `info index` on a project whose `.meta.json` has the Syside shape | the project has no `index`; a Syside KPAR carries `root` and `exports` *(warning on stderr; stdout prints the empty list)* |

### Build

`sysand build` on a Syside project writes the shape above. The two
flags that concern the computed index are refused on a Syside project,
each naming the flag and saying why: `build --keep-index`,
which keeps a hand-maintained `index` map a Syside KPAR does not have,
and `include --no-index-symbols`, which suppresses index computation
for a file that will not be indexed. A workspace build is untouched: a
mixed-family workspace builds each member to its own shape, in one
`output/`, in `members` order, with per-member failure isolation
(workspaces.md, Build output).

### Publish

`sysand publish` accepts a Syside KPAR under these rules (review.md
Q-33's three, and four unchanged from the OMG family):

- `root` is present in `.meta.json` and names a file in the archive
  (P2-08);
- `checksum` covers every file in the archive but the two manifests
  (P2-09), replacing the OMG rule that `checksum` is a superset of the
  `index` files; the empty-`checksum` refusal applies as today;
- `index`, if present, is warned about and ignored (P2-10) -- the same
  rule the reader applies, so that publish does not refuse what every
  other command accepts;
- the checks that today run over every `checksum` key **split by file
  kind**, because the keys now include files that are not model files:
  hash verification runs over every key (a wrong hash, a listed file
  that is missing, an unlisted file that is present -- all as today);
  the source-extension rule (a model file the metamodel does not admit)
  and symbol extraction run over the keys that are model files only. As
  today the rules run over one map; what changes is that `README.md` is
  hashed and not lexed;
- the symbol check keeps its present *direction*, restated for an
  array: every name in `exports` is declared at the top level of some
  model file in the archive, or publish refuses (the
  `NonexistentSymbolExported` refusal, naming the name); a name a file
  declares that `exports` lacks is a warning;
- unchanged: the standard-library checks, and the metamodel
  release-date check.

### Lockfile

The lockfile's format is the same for both families: this schema does
not affect `lock_version`, and the optional `metamodel` field carries
the family. A Syside project's `exports` come from the KPAR's `exports`
array rather than the keys of its `index`, so that the collision check
has the same input for both families.

## JSON to TOML

model-toml.md, JSON to TOML, recovers an OMG project's `sources` as the
union of `index` values and `checksum` keys and errors on a `checksum`
key whose file type the metamodel does not admit. Neither fits a Syside
json project -- the extracted form of a Syside KPAR (see above) -- which
has no `index` and whose `checksum` covers `LICENSES/`, `README.md` and
`CHANGELOG.md`. For a json project whose `.meta.json` declares a Syside
metamodel, `sysand migrate`:

- takes `sources` as the `checksum` keys that are model files (`.kerml`
  or `.sysml`, compared as the extension rule compares them), rewritten
  into the strict form as for an OMG project and deduplicated; the
  other keys are the conventional files a build packages, not an error,
  and a key that is neither a model file nor one of those is dropped
  with a warning naming it;
- takes `root` from `.meta.json`'s `root`, rewritten the same way, and
  refuses with the `model.toml` reader's text if the result is not
  among `sources`;
- refuses a `.meta.json` with neither `index` nor `root` with the
  missing-`root` error, and warns about an `index`, as any reader does.

A model-file `checksum` key the metamodel does not admit is the error
model-toml.md already specifies, since `sources` admits only the types
the metamodel admits.

## Dependencies

model-toml.md, Metamodel, "Third", holds unchanged: the direction rule
(an OMG project must not depend on a Syside one) and the release
warning run over every edge, for both families. Two consequences worth
stating:

- A Syside project depending on the OMG standard library is legal and
  normal. The standard library **stays an OMG project, permanently**
  (the Syside prefix is not one of the standard-library prefixes, and
  the standard library's publisher is `OMG`): the moment a
  standard-library project declared a Syside metamodel, every OMG
  project in existence would fail the direction check against it.
- The cross-project symbol-collision check -- a hard error on every
  resolving path when two dependencies export the same name -- covers
  Syside projects through `exports`. Had the KPAR carried `root` alone,
  a Syside dependency would have arrived exporting nothing and the
  check would have silently stopped covering the family. That is the
  alternative `exports` exists to reject.

A Syside project's source list, for `sysand sources --deps` and for
every command that walks a dependency's files, is its `sources` --
recovered from a KPAR as **the `checksum` keys that are model files**
(`.kerml`/`.sysml`), which equals `sources` by construction. Not every
`checksum` key: under this document's rule the keys include
`LICENSES/`, `README.md` and `CHANGELOG.md`, and a reader that took all
of them -- as the OMG-shape reader does, since for an OMG KPAR
all of them are sources -- would list `README.md` as a source.

## Commands

- **`root` is author-written**, so the manifest editor accepts it as a
  settable and clearable scalar like `version`; whether the value is
  one the project accepts is the reader's question on the next read, as
  for every other scalar. The `sysand info` verb that exposes it on the
  command line does not exist yet (**open**; `info` today gets and sets
  the common keys and `metamodel`). The build-computed fields keep
  their refusal on a toml project: `index`, `exports`, `checksum`,
  `created`. `info index` on a project with the Syside shape prints an
  empty list and says on stderr that there is no `index` (P2-25), so
  that what a script reads from stdout stays a list of symbols; an
  `info exports` verb does not exist yet (**open**).
- **`sysand init --metamodel <syside value>`** is accepted. What it
  writes for a Syside project -- a `root`, and whether a manifest may
  name a file that does not exist yet -- is **open**
  (`implementation-status.md` §3.6). Until it is decided, `init` writes
  what it writes for any project, `sources = []` and no `root`, and the
  next read refuses that file with P2-01.
- **`sysand migrate`** converts a legacy json project to the schema its
  `.meta.json` declares -- the OMG one as today, the Syside one as JSON
  to TOML above says. It does not convert *between* metamodels; how a
  project changes family is **open**.
- **`sysand sources`**, with and without `--deps`, reports `sources` for
  a Syside project and for a Syside dependency as for any other: the
  file list is `sources` for both families (Dependencies above).
- **`sysand include` / `exclude`** are unchanged. Excluding the `root`
  file leaves a `root` that is not an entry, which the next read
  refuses with P2-04; whether `exclude` should refuse to remove the
  `root` instead is **open**.

## Editing the manifest

model-toml.md, Editing the manifest, holds. One rule is this schema's:
a `root` that was read is **written back exactly as it was written**,
whatever it was -- a path, or a value the OMG family ignores. A writer
that normalized or retyped a key it does not judge would change a file
it has no opinion about.

## Non-goals

- Computing reachability from `root`. Stated as a commitment above;
  not a check Sysand makes.
- Per-usage aliases and two coexisting majors of one identity: they
  wait on a language change, and no rule here depends on them.
- A second packaging format (model-toml.md, Non-goals).
- Directories or globs in `sources` (model-toml.md).
