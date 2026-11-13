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
Metamodel). Everything model-toml.md says that is not about `sources`,
which a Syside project replaces with `root`, or about the shape of the
KPAR applies to a Syside project unchanged: names, versions, licences, usages, the standard
library, normalization, editing, the release and direction checks.
This document does not restate them.

Nothing here bumps `manifest-version`. model-toml.md, Manifest version:
"Additions gated by `metamodel` [...] do not bump anything: the
metamodel value already tells a reader which schema applies." A reader that does
not know this schema meets a manifest whose `metamodel` it recognizes
and whose `root` it can name; what it says is under Reading and
validating.

## `root`

A Syside-metamodel project names one source file, its **root**, and
lists no `sources`: its other files are the ones `root` reaches.

```toml
manifest-version = "1.0"

[project]
publisher = "acme"
name = "widgets"
version = "1.0.0"
metamodel = "https://www.sysand.com/spec/SysML/20250201"
root = "Widgets.sysml"
```

- `root` is **required** of every Syside-metamodel project. A Syside
  project without it is an error naming the key and the family (see
  Messages under Reading and validating).
- `root` and `sources` **exclude each other**: a Syside project that
  writes `sources` is an error at the key, whatever it lists (see
  `sources` below).
- `root`'s **syntax is a `sources` entry's**: relative, `/`-separated,
  no `.` or `..` component, no repeated or leading slash, not a
  directory, a `.kerml` or `.sysml` file (model-toml.md, `sources`), and
  in addition never a wildcard entry: `root` names a single file. There
  is one path rule in a manifest, not two. A `root` that breaks a rule
  is an error worded for the key; a `root` that is not a string is the
  type error every other key gets. Whether the file exists is checked
  only where the files are read, as for every `sources` entry.
- Which of `.kerml` and `.sysml` the `root` may be is the metamodel's
  spec kind's decision, exactly as for a `sources` entry (model-toml.md,
  Metamodel, "Second"), with the message worded for `root`.
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

A Syside project's files are the ones `root` reaches through imports,
and `root` is what defines them: there is no list to keep in step with
the files, and nothing in the archive a consumer starting at `root`
cannot reach. **Finding them is not implemented yet**: Sysand's symbol
extractor is a lexer that does not follow imports. Until it is, every
command that needs a Syside `model.toml` project's files or the
symbols they declare refuses the project, naming it and its `root`
(see Commands). A Syside json project (the extracted form of an
archive) is unaffected: its files are the ones its `checksum` lists.

## `sources`

A Syside project lists no `sources`: `root` replaces it. A Syside
`model.toml` that writes `sources` is an error at the key, whatever it
lists, `sources = []` included, so that a manifest says one thing about
its files and not two.

## Reading and validating

model-toml.md, Reading and validating, holds: every key is optional at
parse time, every rule is Sysand's, and "everything that depends on
[`metamodel`] ... [is] checked after inheritance has been resolved". For
this schema, everything about `root` depends on the metamodel:

- **Under a Syside family** -- declared, or inherited from a group --
  `root` is judged: its type, its shape, its language and its presence;
  `sources` is refused. All of it in one report with everything else
  that is wrong, and a `root` that was written and refused is not
  _also_ reported missing.
- **Under the OMG family**, `sources` is required, and `root` is a key
  that "belongs to another
  metamodel's schema" (model-toml.md, Metamodel, first rule): a warning
  naming the family that supports it, and **nothing else, whatever the
  value** (a marker excepted, above). The project is built from
  `sources`; `root` is neither read nor judged, so `root = true` costs
  the same one warning as a path does.
- **Under no family** -- the `metamodel` is missing, or a marker on it
  did not resolve -- nothing is said about `root` (a marker excepted,
  again) or about a missing or present `sources`: the missing metamodel is already in the report, and a
  metamodel that says nothing about the schema decides nothing about
  the key. This is where the `model.toml` reader and the `.meta.json`
  reader differ, and why: `metamodel` is mandatory in a `model.toml`, so
  its absence is already an error, whereas an absent `metamodel` in a
  `.meta.json` _means_ the OMG family (model-toml.md, Metamodel), so the
  json reader warns about `root` there.

A reader that recognizes the Syside values but predates this schema
refuses a Syside project, which has no `sources`, with a message naming
the Syside metamodel as unsupported: the behaviour model-toml.md
promised for a schema a reader does not know.

A member may inherit a Syside metamodel from a group and carry its own
`root`:

```toml
manifest-version = "1.0"

[project]
name = "ports"
publisher.workspace = "syside"
version.workspace = "syside"
metamodel.workspace = "syside"
root = "Ports.sysml"
```

(all three markers name one group, `syside`, which supplies the three
values; `root` is the member's own)

And an OMG project carrying `root` is a valid OMG project with one
warning:

```toml
manifest-version = "1.0"

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

| situation                                                                | text                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Syside project, `root` absent                                            | missing key `root` in `[project]`; the Syside metamodel requires it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `root` breaks a `sources` entry rule                                     | one of six: `root` `<path>` is absolute; `root` is relative to the project root, like a `sources` entry · `root` `<path>` contains a backslash `\`; it is `/`-separated, so that a manifest reads the same way on every operating system · `root` `<path>` contains a NULL character · `root` `<path>` is not in the form a `sources` entry takes: relative, `/`-separated, with no `.` or `..` component and no repeated or leading slash · `root` `<path>` names a directory; `root` names a single model file · `root` `<path>` is not a KerML (`.kerml`) or SysML (`.sysml`) file |
| `root` is not a string                                                   | `root` in `[project]` must be a string, but it is `<type>`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `root`'s extension is not admitted by the metamodel                      | `root` `<path>` is a `<language>` file, which the metamodel `<url>` does not admit; a KerML project lists `.kerml` files, a SysML project `.sysml` and `.kerml` files                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `root` contains `*`                                                      | `root` `<path>` contains `*`; it must name a single file                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `root` in an OMG-metamodel project                                       | `root` in `[project]` is only supported by the Syside metamodel and is ignored here _(warning)_                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `root.workspace = true`                                                  | `root` in `[project]` cannot be inherited from the workspace; it is a per-project entry point                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Syside project, `sources` present                                        | `sources` in `[project]` is not used by the Syside metamodel; a Syside project names `root`, and its other files are reached from it                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| a command needs a Syside `model.toml` project's files or symbols         | `<publisher>/<name>` is a Syside-metamodel project, whose files are reached from its `root` `<path>`; finding them is not supported yet                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `info metamodel` switching to a Syside metamodel with no `root`          | changing the metamodel to `<url>` requires `root`; `model.toml` does not have it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `include` or `exclude` on a Syside `model.toml` project                  | a Syside-metamodel project lists no `sources`; its files are reached from its `root`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `info metamodel` switching to a Syside metamodel with files in `sources` | `` `sources` lists <n> file(s), and a Syside project lists none; `sysand exclude` removes them `` _(an empty `sources` is removed)_                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `info metamodel` switching to an OMG metamodel with a `root` present     | `root` is only supported by the Syside metamodel and will be ignored under `<url>`; `sysand info root --clear` removes it _(warning; the switch is written, with `sources = []`)_                                                                                                                                                                                                                                                                                                                                                                                                     |
| `info exports` on a project whose `.meta.json` has the OMG shape         | the project has no `exports`; an OMG KPAR carries `index` _(warning on stderr; stdout prints the empty list)_                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

The multi-line texts are shown with their line breaks as `;`, as a
parenthesis, or as a space after a comma; the exact bytes are the
tests'.

## The Syside KPAR

_One question in this section is open: how format detection classifies
the extracted form beside a `model.toml`. Detection by file presence
treats it as the json project it is._

**Not accepted yet.** The format below is not final. Every reader of a
`.meta.json` (a json project, a KPAR, an index entry) refuses one that
declares a Syside metamodel, unless Sysand is built with the
non-default `syside-kpar` feature; the shape is still judged, and its
problems reported beside the refusal. Shipped builds do not enable the
feature. A Syside `model.toml` project is unaffected: it is read, and
the commands that need its files refuse it until they can be found.

A KPAR built from a Syside-metamodel project carries `.project.json` and
`.meta.json` as any KPAR does (model-toml.md, TOML to KPAR), and never
`model.toml`. `.project.json` is unchanged. `.meta.json` differs from an
OMG KPAR's in three ways:

- **`root`** is carried, as written in the manifest -- the same path
  that keys its `checksum` entry.
- **`exports`** replaces `index`: a flat array of the top-level symbols
  declared by every model file in the archive, computed at build time
  by the same extractor that fills an OMG KPAR's `index` map, each name
  once, sorted -- so that two builds of the same files write the same
  array whatever order the files were read in, which `sources` order
  cannot promise for the extracted form, where there is no `sources`.
  It is an array of names, not a map of name to file: the map is what
  the Oxide consumers need collapsed, and the file for every symbol is
  `root`, by construction, for a consumer resolving through it. Sysand needs the
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
`root` and the files it reaches.

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
a missing-`index` error that names no metamodel. A reader that knows
this schema names the Syside metamodel, as the `model.toml` reader
names it. Every consumer that embeds `sysand-core` and reads
`.meta.json` must ship a core that knows this shape before any Syside
KPAR is published.

| situation                                                                             | text                                                                                                                                                            |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `build --keep-index`, or `include --no-index-symbols`, on a Syside project            | `<flag>` applies to projects with a computed index; a Syside project's KPAR carries `root` and `exports` instead                                                |
| publish: `root` names a file that is not in the archive                               | the archive does not contain `root` `<path>`                                                                                                                    |
| publish: `checksum` misses a file                                                     | `checksum` does not cover `<path>`; a Syside KPAR checksums every file in the archive but `.project.json` and `.meta.json`                                      |
| a Syside `.meta.json` carrying `index`                                                | `index` in `<file>` is not used by the Syside metamodel and is ignored; a Syside KPAR carries `root` and `exports` _(warning, wherever the file is read)_       |
| `.meta.json` with no `root` under a Syside metamodel; with no `index` under any other | missing key `root` in `<file>`; the Syside metamodel requires it _-- and under any other family:_ missing key `index` in `<file>`                               |
| `.meta.json` carrying `root` or `exports` under an OMG or absent metamodel            | `<key>` in `<file>` is only supported by the Syside metamodel and is ignored here _(warning, one per key; the `model.toml` text with the file as the location)_ |
| `.meta.json` with no `exports` under a Syside metamodel                               | missing key `exports` in `<file>`; the Syside metamodel requires it                                                                                             |
| `info index` on a project whose `.meta.json` has the Syside shape                     | the project has no `index`; a Syside KPAR carries `root` and `exports` _(warning on stderr; stdout prints the empty list)_                                      |
| publish: `exports` names a symbol no model file in the archive declares               | metadata claims the project exports `<symbol>`, but no file in the archive declares it; the archive declares `<names>` _(or "nothing")_                         |
| publish: `.project.json` or `.meta.json` is a `checksum` key                          | `checksum` covers `<path>`, which a Syside KPAR does not checksum                                                                                               |

### Build

`sysand build` on a Syside project writes the shape above. A Syside
`model.toml` project is refused until the files `root` reaches can be
found (see What `root` is for); the extracted form builds. The two
flags that concern the computed index are refused on a Syside project,
each naming the flag and saying why: `build --keep-index`,
which keeps a hand-maintained `index` map a Syside KPAR does not have,
and `include --no-index-symbols`, which suppresses index computation
for a file that will not be indexed. A workspace build is untouched: a
mixed-family workspace builds each member to its own shape, in one
`output/`, in `members` order, with per-member failure isolation
(workspaces.md, Build output).

### Publish

`sysand publish` accepts a Syside KPAR under these rules:

- `root` is present in `.meta.json` and names a file in the archive;
- `checksum` covers every file in the archive but the two manifests,
  in place of the OMG rule that `checksum` is a superset of the `index`
  files; the empty-`checksum` refusal applies as for an OMG KPAR;
- `index`, if present, is warned about and ignored -- the same rule the
  reader applies, so that publish does not refuse what every other
  command accepts;
- the checks over the `checksum` keys **split by file kind**, because
  the keys include files that are not model files: hash verification
  runs over every key (a wrong hash, a listed file that is missing, an
  unlisted file that is present -- all as for an OMG KPAR); the
  source-extension rule (a model file the metamodel does not admit) and
  symbol extraction run over the keys that are model files only. The
  rules run over one map; `README.md` is hashed and not lexed;
- neither manifest is a `checksum` key: they are the exception to
  "every file", and since neither is a model file the extension rule
  would not catch a document that listed them;
- the symbol check has the OMG family's _direction_, restated for an
  array: every name in `exports` is declared at the top level of some
  model file in the archive, or publish refuses (naming the name and
  what the archive does declare); a name a file declares that
  `exports` lacks is a warning. There is no empty-`exports` warning: a
  Syside project has at least one source, since `root` is one, and the
  empty-`checksum` refusal already guards an archive with no files;
- order, for an archive with more than one thing wrong: `root` absent
  from the archive is the missing-`root` refusal whether or not
  `checksum` lists it; `root` present but not listed is the
  uncovered-file refusal, like any other file;
- unchanged: the standard-library checks, and the metamodel
  release-date check.

### Lockfile

The lockfile's format is the same for both families: this schema does
not affect `lock_version`, and the optional `metamodel` field carries
the family. A Syside project's `exports` come from the KPAR's `exports`
array rather than the keys of its `index`, so that the collision check
has the same input for both families. A Syside `model.toml` project's
exports are the symbols of the files `root` reaches, so locking one is
refused until those can be found.

## JSON to TOML

model-toml.md, JSON to TOML, recovers an OMG project's `sources` as the
union of `index` values and `checksum` keys and errors on a `checksum`
key whose file type the metamodel does not admit. Neither fits a Syside
json project -- the extracted form of a Syside KPAR (see above) -- which
has no `index` and whose `checksum` covers `LICENSES/`, `README.md` and
`CHANGELOG.md`. For a json project whose `.meta.json` declares a Syside
metamodel, `sysand migrate`:

- takes `root` from `.meta.json`'s `root`, and writes no `sources`: a
  Syside `model.toml` lists none, and its files are the ones `root`
  reaches. The `checksum` keys are not read;
- refuses a `.meta.json` with no `root` with the missing-`root` error,
  naming `.meta.json`, and warns about an `index`, as any reader does.

`exports`, like `index`, is not migrated: the names are computed when
the project is built, and `migrate` says so.

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

A Syside dependency's source list, for `sysand sources --deps` and for
every command that walks a dependency's files, is recovered from a KPAR
as **the `checksum` keys that are model files** (`.kerml`/`.sysml`). Not every
`checksum` key: under this document's rule the keys include
`LICENSES/`, `README.md` and `CHANGELOG.md`, and a reader that took all
of them -- as the OMG-shape reader does, since for an OMG KPAR
all of them are sources -- would list `README.md` as a source.

## Commands

- **`root` is author-written**, so `sysand info root` gets it, and
  `--set <path>` and `--clear` write it: on a `model.toml` project
  through the manifest editor like `version`, on a json project in
  `.meta.json` like `metamodel`. Whether the value is one the project
  accepts is the reader's question on the next read, as for every
  other scalar. **`sysand info exports`** lists what the project's KPAR
  exports -- computed for a `model.toml` project, read for a json one
  -- and, on a project with no `exports` (the OMG shape), prints an
  empty list and says so on stderr, the mirror of `info index` on the
  Syside shape; neither can be set. The other
  build-computed fields keep their refusal on a toml project:
  `checksum`, `created`.
- **Changing the metamodel** with `sysand info metamodel` (a Syside
  value is reached through `--set-custom <url>`): a switch to a Syside
  metamodel on a project with no `root` is refused and the manifest is
  left as it was -- set `root` first; it removes an empty `sources`,
  and is refused while `sources` lists files. A switch to an OMG
  metamodel writes `sources = []` where there is none, and on a project
  with a `root` warns that `root` is now a foreign key. The required-key consequences are thus reported at the
  switch rather than found on the next read.
- **`sysand init --metamodel <syside value>`** writes the Syside
  layout: `root` naming one file named after the project
  (`<normalized name>.sysml`, or `.kerml` for the KerML metamodel), no
  `sources`, and the file itself -- one line, `package <name>;`,
  the name quoted when it is not an identifier -- created when it does
  not exist and left alone when it does. Inside a workspace whose
  default group declares a Syside metamodel, the marker is written and
  the layout still follows the group's value. Because the file is
  created, a generated project's `root` names a file that is there.
- **`sysand migrate`** converts a json project to the schema its
  `.meta.json` declares -- the OMG one as model-toml.md says, the
  Syside one as JSON to TOML above says. It does not convert _between_
  metamodels: a project changes family with `sysand info metamodel`,
  above.
- **`sysand sources`** refuses a Syside `model.toml` project, whose
  files cannot be found yet; for a Syside dependency it reports the
  model files its archive checksums (Dependencies above).
- **`sysand build`**, **`sysand lock`** (and so `sync`), and **`sysand
info exports`** likewise refuse a Syside `model.toml` project, with
  the message in Messages.
- **`sysand include` / `exclude`** refuse a Syside `model.toml`
  project, which lists no `sources`. On the extracted form,
  `include --no-index-symbols` is refused with the text
  `build --keep-index` gets, which says the project has no `index`.

## Editing the manifest

model-toml.md, Editing the manifest, holds. One rule is this schema's:
a `root` that was read is **written back exactly as it was written**,
whatever it was -- a path, or a value the OMG family ignores. A writer
that normalized or retyped a key it does not judge would change a file
it has no opinion about.

### Editor messages

Three messages of the `model.toml` _editor_ -- not of this schema, but
listed here because the verb that meets them (`set_usage_constraint`,
in the bindings) has a `model.toml` arm. A `model.toml` usage is
addressed by identity; only an index usage has a constraint; and an
index usage requires one.

| situation                                                                         | text                                                                                                                 |
| --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| clearing the constraint of a `model.toml` index usage                             | a `model.toml` index usage needs a version constraint                                                                |
| setting the constraint of a `dir`, `kpar-path`, `kpar-url`, `git` or opt-in usage | `<publisher/name>` is not an index usage and has no version _(the editor's text, also reached through `sysand add`)_ |
| addressing a `model.toml` usage by an IRI that is not a `pkg:sysand` PURL         | `<iri>` is not a `pkg:sysand` PURL; a `model.toml` usage is named `publisher/name`                                   |

## Non-goals

- Computing reachability from `root`, for now: until it exists, the
  commands that need a Syside `model.toml` project's files refuse it
  (What `root` is for).
- Per-usage aliases and two coexisting majors of one identity: they
  wait on a language change, and no rule here depends on them.
- A second packaging format (model-toml.md, Non-goals).
- Directories or globs in `sources`: closed, not deferred (model-toml.md:
  every file is listed explicitly).
