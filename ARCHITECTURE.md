# Architecture

This project provides a Rust Library, a Rust based CLI (`sysand` binary), and
bindings for Python, Java, and JS/WASM. The Rust library is the most
foundational part, the CLI and bindings rely on it.

```
  ┌────────────────┐     ┌─────────────┐
  │ sysand-macros  │────>│ sysand-core │
  │ proc macros    │     │ Rust lib    │
  └────────────────┘     └──────┬──────┘
              ┌───────────┬─────┴────────┬────────────────┐
              v           v              v                v
         ┌────────┐ ┌────────────┐ ┌─────────────┐ ┌─────────────┐
         │ sysand │ │ sysand-py  │ │ sysand-java │ │ sysand-js   │
         │ CLI    │ │ Python lib │ │ Java lib    │ │ JS/WASM lib │
         └────────┘ └────────────┘ └─────────────┘ └─────────────┘
```

The CLI and each language binding are **surfaces** — different ways to access the
same core functionality. The goal is to keep these surfaces aligned so that
users get a consistent experience regardless of language, while keeping the
codebase easy to understand and sustainable to maintain. In practice this is
complicated by [FFI] constraints (e.g. generics can't cross language boundaries),
differing async models, and varying levels of platform support (the JS/WASM
surface has no filesystem or networking).

[FFI]: https://en.wikipedia.org/wiki/Foreign_function_interface

## Repository structure

The whole repository is a [Cargo workspace] composed of multiple crates (Rust
packages) and various other language libraries that wrap the Rust core.

Directory structure:

- `core` contains the `sysand-core` crate with all the core logic referenced by
  the bindings and `sysand` CLI.
- `sysand` contains the `sysand` crate, and wraps `sysand-core` into the
  `sysand` CLI.
- `bindings` contains wrappers of `sysand-core` to provide various programming
  languages with an API to use sysand:
  - `bindings/py` contains the `sysand-py` crate, providing a Python library.
  - `bindings/java` contains the `sysand-java` crate, providing a Java library.
  - `bindings/js` contains the `sysand-js` crate, providing a WASM/JavaScript
    library that can be used in Node, Deno, browsers, etc.
- `macros` contains the `sysand-macros` crate used sparsely by `sysand-core`,
  providing `#[derive(ProjectRead)]` and `#[derive(ProjectMut)]` procedural
  macros.

  As of 2026-04, only `#[derive(ProjectRead)]` is used, and only for
  `AnyProject` and `CombinedProjectStorage`.

The non-Rust bindings are in an early state of development. The JavaScript/WASM
library in particular is only a proof-of-concept that is not yet usable.

[cargo workspace]: https://doc.rust-lang.org/cargo/reference/workspaces.html

## Key concepts

Sysand works with [SysML v2] and [KerML] Interchange Projects, as defined in the KerML spec section 10.3, and also introduces a few concepts of its own.

```
Workspace (.workspace.json)
 └─ lists projects with their paths and IRIs
 └─ optionally specifies a metamodel (SysML/KerML)

Environment (storage backend)
 └─ stores projects indexed by IRI and version
 └─ implementations: local directory, remote index, git, in-memory

Project (Interchange Project)
 └─ .project.json  — name, version, publisher, usages (dependencies), etc.
 └─ .meta.json     — source file index, checksums, timestamps
 └─ source files   — the actual SysML/KerML model files

Lock file (sysand-lock.toml)
 └─ resolved dependency graph with pinned versions, checksums, and sources
```

[SysML v2]: https://www.omg.org/spec/SysML/2.0
[KerML]: https://www.omg.org/spec/KerML

## Core traits

The core library uses trait-based abstractions for reading and writing projects
and environments. Each trait has an associated error type, which allows
implementations to define their own error types while keeping the trait generic.

### ProjectRead and ProjectMut

`ProjectRead` is the trait for reading project data (`get_project`,
`read_source`, `sources`). Each implementation defines its own error and
reader types.

`ProjectMut` extends `ProjectRead` with write operations (`put_info`,
`put_meta`, `write_source`, etc.).

Notable implementations include `LocalSrcProject` (directory on disk; also
implements `ProjectMut`), `LocalKParProject` (KPAR/ZIP archive),
`InMemoryProject` (also implements `ProjectMut`), and async variants for HTTP
and git sources. Wrapper types like `EditableProject<P>`, `CachedProject<Local,
Remote>`, and `AnyProject<Policy>` compose these.

### ReadEnvironment and WriteEnvironment

`ReadEnvironment` provides access to a collection of
projects by URIs and version (`uris`, `versions`, `get_project`).

`WriteEnvironment` adds `put_project` (using a callback pattern so the
environment can handle allocation and cleanup), `del_project_version`, and
`del_uri`.

Notable implementations include `LocalDirectoryEnvironment` (filesystem),
`MemoryStorageEnvironment<Project>` (in-memory), and
`IndexEnvironmentAsync<Policy>` (remote sysand index; see
"Index environment" below).

### Async variants

Some project and environment implementations are inherently async (HTTP, git)
while others are sync (local filesystem, in-memory). Both need to be usable in
either context, so the read traits have async counterparts (`ProjectReadAsync`,
`ReadEnvironmentAsync`).

Adapter types bridge the two: `AsAsyncEnvironment<T>` wraps a sync
implementation for use in async contexts, and `AsSyncEnvironmentTokio<T>` wraps
an async implementation for sync callers via a tokio runtime. There is no async
variant of `ProjectMut` or `WriteEnvironment`, because these are only used for local/in-memory projects/environments.

## The Raw type pattern

Many types exist in two forms: a **raw** form and a **validated** form. Both are
type aliases of a shared generic base struct (suffixed with `G` for "Generic"):

```
  *Raw  (all String fields, JSON-compatible)
    │
    ├── TryFrom<Raw> ──> *Validated  (parsed semantic types: Iri, Version, …)
    └── From<Validated> ─── back to Raw
```

This pattern appears in `InterchangeProjectUsageG`, `InterchangeProjectInfoG`,
`InterchangeProjectMetadataG`, `WorkspaceInfoG`, `WorkspaceMetaG`, and
`WorkspaceProjectInfoG`.

**Trait boundaries mostly use Raw types.** For example, `ProjectRead::get_project()` returns
`InterchangeProjectInfoRaw`, not the validated form. This is a deliberate
design choice: traits work with the lowest-common-denominator representation,
and callers validate when they need to.

## Feature flags

The `sysand-core` crate uses feature flags to control what capabilities are
compiled in:

| Feature          | What it enables                                                              |
| ---------------- | ---------------------------------------------------------------------------- |
| `std` (default)  | Standard library support in dependencies. Must be enabled.                   |
| `python`         | PyO3 dependency; enables `FromPyObject`/`IntoPyObject` derives on core types |
| `js`             | `wasm-bindgen` dependency for WASM/JavaScript support                        |
| `filesystem`     | Local file I/O, KPAR reading/writing (`zip`, `camino-tempfile`, `dirs`)      |
| `networking`     | HTTP requests and git cloning (`reqwest`, `gix`)                             |
| `lenient_checks` | More lenient validation where the KerML spec is vague                        |
| `kpar-*`         | Additional KPAR compression methods (bzip2, zstd, xz, ppmd)                  |

Each binding crate enables different features on `sysand-core` (all also get
`std` via default features):

| Crate          | Features enabled on `sysand-core`                         |
| -------------- | --------------------------------------------------------- |
| `sysand` (CLI) | `filesystem`, `networking` (+ optional `kpar-*`)          |
| `sysand-py`    | `python`, `filesystem`, `networking`                      |
| `sysand-java`  | `filesystem`, `networking`                                |
| `sysand-js`    | `js` only (no filesystem or networking — runs in browser) |

## Bindings architecture

### The monomorphization requirement

While the Rust CLI can consume the Rust library's generic types directly, the
language bindings cannot. A Python function can't be mapped to a generic Rust
function - the types must be concrete at the FFI boundary. This is why the [Raw
type pattern](#the-raw-type-pattern) is important: all trait methods return
`*Raw` types (all-`String` fields), which are easy to convert to and from any
language.

### How each binding works

**Python** (PyO3) has the most flexibility. The `python` feature flag enables
`#[derive(FromPyObject, IntoPyObject)]` on core types, so PyO3 can
automatically convert between Python objects and Rust structs. At the
`#[pyfunction]` boundary, the binding accepts Python strings, manually parses
them into semantic types (e.g. `VersionReq::parse()`), and returns `*Raw` types
that PyO3 auto-converts to Python objects.

**Java** (JNI) is fully manual. The binding defines a custom `ToJObject` trait
with hand-written implementations for each `*Raw` type. Each conversion
constructs a Java object field by field via JNI calls.

**JavaScript/WASM** (`wasm-bindgen`) is the most restrictive — `wasm-bindgen`
cannot annotate generic types at all. The JS binding implements `ProjectRead`
and `ProjectMut` on its own concrete types (e.g. `ProjectLocalBrowserStorage`)
and only works with `*Raw` types. The JS binding also has no filesystem or
networking support (it runs in the browser), so it provides its own
browser-compatible storage layer.

### Error handling across bindings

Each binding translates Rust error types into the target language's exception
system:

- **Python**: Pattern-matches on error variants and maps them to PyO3 exception
  types (`PyIOError`, `PyValueError`, `PyFileExistsError`, etc.)
- **Java**: Defines custom `ExceptionKind` enum types and throws them via JNI
  (`throw_exception`), with a separate `StdlibExceptionKind` for standard Java
  exceptions
- **JavaScript**: Converts all errors to `JsValue` via their `Display`
  implementation (no type-based differentiation)

## Sysand specific files

### `.workspace.json`

Defines a workspace. Lists the projects the user is working on and optionally
specifies a default metamodel for projects' `.meta.json` files:

```json
{
  "projects": [{ "path": "my-project", "iris": ["urn:example:my-project"] }],
  "meta": {
    "metamodel": "https://www.omg.org/spec/SysML/20250201"
  }
}
```

### `sysand.toml`

Defines sysand configuration, and has [user facing configuration
documentation](docs/src/config.md).

The sysand CLI is by default merging a user-level sysand config from
`$XDG_CONFIG_HOME/sysand/sysand.toml` or a platform equivalent location with a
project-local `sysand.toml`.

It is as of 2026-04 not yet clearly defined if/how bindings should consider
project-local and optionally user-level configuration.

### `sysand-lock.toml`

Captures a project's resolved usages and their dependencies.

The `sysand lock` command regenerates this file, recording each project's name,
version, exported symbols, dependency usages, sources (local paths, index URLs,
git repos, etc.), and a content checksum. The `sysand sync` command reads
`sysand-lock.toml` to populate `sysand_env`, and will run `lock` first if the
file does not yet exist.

### Local environment (`sysand_env`)

A local environment for use by tools like `syside`. It can be initialized by
`sysand env`, and populated with `sysand sync`.

The local environment looks like this:

```text
sysand_env
 ├──env.toml
 └──lib
    ├──package_ID1_version
    ├──package_ID2_version
    └──package_ID3_version
```

`env.toml` contains an array of projects. Each project contains these fields:

- `publisher`. Taken from `.project.json`
- `name`. Taken from `.project.json`
- `version`. Taken from `.project.json`
- `path`. Relative path of the project's directory. For non-`workspace` and
  non-`editable` projects, this is currently `lib/package_ID_version`, and is
  relative to `sysand_env`. For `editable`/`workspace` projects, path is
  relative to the workspace/project root; these projects are only listed in
  `env.toml`, but are not otherwise managed by it.
- `identifiers`. Identifiers of the project, the first one being considered
  "canonical".
- `usages`. Taken from `.project.json`
- `editable`. Whether the project is considered to be "editable". If so,
  the project entry is only a reference to that project, and the project
  itself is not managed by the environment.
- `workspace`. Whether the project belongs to the current workspace.
  `workspace` projects are always `editable`.

Each `package_ID` is derived from the project's canonical identifier, and
`version` is derived from project's version. For exact details on how each
`package_ID` and `version` is constructed from their corresponding values, see
`sysand_core::env::iri_normalize::iri_to_filename_part()` and
`sysand_core::env::iri_normalize::normalize_version()`, respectively.

### Index environment

An index environment is sysand's client-side reader for sysand index
servers. The **wire contract** — directory layout, `index.json`,
`versions.json`, per-version files, IRI→path resolution, required digests,
and server/client obligations — is specified in
[`docs/src/index-protocol.md`](docs/src/index-protocol.md) and deliberately
not duplicated here.

Index tree creation and mutation are owned by the `sysand index` command
group described in the protocol docs. Publishing uses the discovered
`api_root`.

#### Client implementation notes

Sysand-core specifics that aren't part of the protocol:

- `IndexEnvironmentAsync` is the client implementation; its per-version
  leaf is `IndexEntryProject`. Advertised-version reads return
  `versions.json` fields with no I/O (`version_async`, `usage_async`);
  `checksum_canonical_hex_async` returns the advertised digest until the
  archive has been verified, after which it checks the archive-backed
  digest. Per-version `.project.json` / `.meta.json` are fetched once
  behind an internal `OnceCell`, and `project.kpar` is verified against
  the advertised `kpar_digest` during download.
- Because the kpar isn't on disk during resolution, `Source::IndexKpar`
  populates `index_kpar_size` and `index_kpar_digest` directly from
  `versions.json`, so lockfile writing records archive metadata without a
  HEAD round-trip.
