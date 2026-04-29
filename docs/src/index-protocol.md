# Sysand Index Protocol

> **Status: v0, unversioned.** This document is not yet a stable contract.
> Breaking changes between sysand releases are expected until v1.

## 1. Scope

This document specifies the **sysand index** — the tree of files a sysand
index server exposes over HTTP. It covers file layout, JSON shapes,
discovery, the obligations an index server must uphold (§11), and the
verification rules clients apply (§12). It doesn't cover authentication or
authorization.

A _sysand index server_ is any service that hosts a sysand index. It MAY
additionally expose a _sysand index API_ (publish, yank, and other
management endpoints); that API is specified separately in the
[sysand index API protocol](index-api-protocol.md) and is not covered here.

The keywords MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD
NOT, RECOMMENDED, MAY, and OPTIONAL are to be interpreted as described in
[RFC 2119][rfc2119].

## 2. Implementability

A sysand index MUST be serveable from an ordinary HTTP static-file server, i.e.
files can be served as-is from disk without server-side computation
requirements.

A smart server MAY respond to requests for `.project.json`, `.meta.json`,
or `project.kpar` with HTTP redirects (e.g. to an object store or CDN);
clients MUST follow such redirects. The digest commitments in
[§10] provide end-to-end integrity
regardless of transport.

The `sysand index` CLI command group produces and maintains index trees.

## 3. Discovery and configuration

The URL the user configures — the **index URL** in user-facing terms — is
the protocol's **discovery root**. It is not necessarily the root at which
index files are served.

On first contact, the client fetches:

```text
<discovery-root>/sysand-index-config.json
```

Clients normalize a configured discovery root to a directory URL before
joining that path: `https://example.org/index` is treated as
`https://example.org/index/`, so discovery fetches
`https://example.org/index/sysand-index-config.json`.

If present (HTTP 200), the response is a JSON object with these optional
fields:

```json
{
  "index_root": "https://sysand.org/index/",
  "api_root": "https://sysand.org/api/"
}
```

- `index_root` — base URL of the sysand index (where `index.json` lives).
  When absent, defaults to the discovery root.
- `api_root` — base URL of the sysand index API (where `v1/upload` and
  other endpoints live). When absent, defaults to the discovery root.

`index_root` and `api_root`, when present, MUST be absolute `http` or
`https` URLs ([RFC 3986 §4.3][rfc3986-43]: scheme + hier-part, no
relative references) and MUST NOT contain URL userinfo (`username` or
`password`). Clients MUST reject a discovery document that supplies a
relative URL or userinfo for either field rather than attempting to
resolve it against the discovery root or the final URL of the
discovery-document fetch — relative URLs are excluded to avoid ambiguity
around the resolution base after redirects, and userinfo is excluded so
credentials are not logged, persisted, or propagated through generated
source URLs.

If the discovery document is absent (HTTP 404) the client proceeds as
though it were present with no fields set: `index_root` and `api_root`
both default to the discovery root. Any other non-success response (e.g.
5xx) is a hard error.

Clients MUST follow HTTP redirects on the discovery fetch. Unknown fields
in the document are silently ignored (see [§14]).

## 4. Layout

Anchored at `index_root`, a sysand index is a tree:

```text
<index_root>/
├── index.json
├── _iri/
│   └── <sha256_hex(normalized_iri)>/
│       ├── versions.json
│       └── <version>/
│           ├── .project.json
│           ├── .meta.json
│           └── project.kpar
└── <publisher>/
    └── <name>/
        ├── versions.json
        └── <version>/
            ├── .project.json
            ├── .meta.json
            └── project.kpar
```

A project lives in exactly one of the two trees — `_iri/...` or
`<publisher>/<name>/...` — depending on its IRI shape
([§5]).

## 5. IRI → path resolution

Given a project IRI, clients resolve the project directory as follows:

- If the IRI matches `pkg:sysand/<publisher>/<name>` and both segments
  satisfy the canonicalization rules in
  [§6], the project directory is
  `<index_root>/<publisher>/<name>/`.
- Otherwise, the project directory is
  `<index_root>/_iri/<sha256_hex(normalized_iri)>/`, where
  `sha256_hex(normalized_iri)` is the lowercase hex SHA-256 of the IRI
  after applying the normalization defined in
  [§5.1].

Clients MUST reject `pkg:sysand/` IRIs that are not canonical
([§6]).

### 5.1. IRI canonicalization for the `_iri` hash bucket

Before hashing, the IRI MUST be canonicalized by applying the following
steps in order. The intent is to produce identical hashes for all URLs that
are canonically equivalent, i.e. refer to the same resource:

1. **Syntax-based normalization** — apply
   [`fluent_uri::Iri::normalize`][fluent-uri-normalize] semantics. This
   delegates syntax-based IRI normalization, including percent-encoding,
   case, port, dot-segment, and IPv6 literal handling, to `fluent-uri`.
2. **Host → Punycode** — if the authority host is a
   [registered name][rfc3986-reg-name] (a domain-name-like host) containing
   non-ASCII characters, replace it with the result of
   [`domainToASCII`][whatwg-url-domain-to-ascii]. IPv4/IPv6 literals are not affected.
3. **HTTP root path** — if the scheme is `http` or `https` and the path
   is empty, replace the empty path with `/`
   ([RFC 9110 §4.2.3][rfc9110-423]).

Two IRIs that yield the same byte sequence after steps 1–3 are the same
project for the purposes of this protocol; any other difference yields a
different project even if a scheme-specific interpretation would consider
them equivalent.

## 6. Sysand PURL relation to canonicalization

Sysand PURL (TODO: link to https://packageurl.org/) is of the form `pkg:sysand/<publisher>/<name>`. It is intended
to identify projects in a consistent way. In the index
protocol, no other standard PURL components (version/build) are permitted.
Publisher and name are of the form:

- Length: 3–50 ASCII characters.
- Starts and ends with an ASCII lowercase letter or digit.
- Between the first and last character: ASCII lowercase letters, digits,
  and isolated separators. No two separators may be adjacent.
- Allowed separators:
  - Publisher: hyphen (`-`).
  - Name: hyphen or dot (`.`).

PURL will be used to identify all projects whose `publisher`
and `name` each satisfy:

- Length: 3–50 ASCII characters.
- Starts and ends with an ASCII letter or digit.
- Between the first and last character: ASCII letters, digits,
  and isolated separators. No two separators may be adjacent.
- Allowed separators:
  - Publisher: hyphen (`-`) or space.
  - Name: hyphen, space or dot (`.`).

To create the PURL, publisher and name are normalized by lowercasing
and replacing spaces with hyphens.

Once a PURL is created, it is considered canonical and no further
normalization is done. Any Sysand PURL not satisfying the Sysand PURL rules
above is considered invalid and must be rejected without any further
processing.

## 7. `index.json`

Served at `<index_root>/index.json`. A flat enumeration of every project
IRI the index knows about:

```json
{
  "projects": [
    { "iri": "pkg:sysand/abc/def" },
    { "iri": "https://example.org/project.kpar" }
  ]
}
```

Today, clients use `index.json` when enumerating every IRI an index
advertises through `ReadEnvironment::uris` / `uris_async`. Resolving a
specific IRI does not consult `index.json`; it fetches that project's
`versions.json` directly ([§8]).

An empty index has `index.json` that contains `{ "projects": [] }`.

A 404 on `index.json` is a hard error, i.e. the URL is assumed to not
point to an index.

## 8. `versions.json`

Served at the project directory ([§5]). Lists every version of the project
along with the metadata a client needs to solve without fetching any
per-version file:

```json
{
  "versions": [
    {
      "version": "2.3.4",
      "usage": [
        { "resource": "pkg:sysand/abc/dep", "versionConstraint": "<2" }
      ],
      "project_digest": "sha256:<64-hex>",
      "kpar_size": 12345,
      "kpar_digest": "sha256:<64-hex>"
    }
  ]
}
```

Per-entry rules:

- All five fields (`version`, `usage`, `project_digest`, `kpar_size`,
  `kpar_digest`) are REQUIRED. A client MUST reject a `versions.json`
  that omits any of them.
- `version` MUST parse as a [semver 2.0.0][semver] version and MUST NOT
  carry build metadata (`+<something>`). Pre-release identifiers
  (`-beta.1` etc.) are permitted. Build metadata is excluded because
  semver specifies that two versions differing only in their build
  metadata have equal precedence (semver §10); permitting it in the
  index would create two entries that sort identically but address
  different artifacts, breaking the `(iri, version)` identity contract
  (see [§13]).
- `usage` is an array of dependency declarations in the same shape as in
  `.project.json`. It duplicates the version's project manifest so the
  solver can run from `versions.json` alone.
- `project_digest` and `kpar_digest` are lowercase SHA-256 in
  `sha256:<64-hex>` form ([§10]).
- `kpar_size` is the byte length of the archive.
- `status` is OPTIONAL. When present, it MUST be one of `"available"`,
  `"yanked"`, or `"removed"`; an omitted `status` is equivalent to
  `"available"`. Servers SHOULD omit the field when its value would be
  `"available"` to save space. Clients MUST
  accept both the omitted form and an explicit `"available"` as
  equivalent. `yanked` and `removed` entries are collectively
  "retired"; see [§11] for the server obligations they impose and
  [§12] for client behaviour.

Ordering:

- Entries MUST appear in descending order of parsed semver precedence
  (newest-first).

Duplicates:

- `versions.json` MUST NOT list the same version twice.

Absence:

- A `versions.json` 404 means the project is not in this index.
  Clients query indexes by IRI without first consulting
  `index.json`, and each index hosts only some IRIs, so a 404 here
  is the ordinary "look elsewhere" signal — not a protocol
  violation. A client MUST treat the 404 as "not in this index";
  in a resolver chain it continues to the next source, and the
  operation fails only when no source returns a usable `versions.json`
  and no source returns a hard error. Non-404 errors remain hard
  errors. This applies to `get_project` as well: a 404 on that
  project's `versions.json` reports "not in this index" to the direct
  caller, who decides whether that is fatal.
- A project that the index lists in `index.json` but for which no
  version has been published is represented by a 200 response with
  `{ "versions": [] }`. Servers MUST serve a `versions.json` for
  every project listed in `index.json` ([§11]); the 404 case
  therefore corresponds to a project the index does not list.

## 9. Per-version files

Each version in `versions.json` has a subdirectory under the project
directory ([§4]) whose name is byte-identical to the entry's `version`
field (e.g. `0.3.0/`, `10.0.0-beta.1/`). That directory MUST contain:

- `.project.json` — interchange project info.
- `.meta.json` — interchange project metadata.
- `project.kpar` — the archive.

A version's file presence is governed by its [§8] `status`:

- `available` and `yanked` — all three files MUST be retrievable;
  a 404 on any of them is a hard error.
- `removed` — all three files MUST 404. A client that has just read the
  corresponding `versions.json` entry MUST reject the version before
  fetching these files and surface it as a distinct removed-upstream
  error. A lockfile `sync` starts from the recorded archive URL and
  digest and does not re-read `versions.json`, so a removed archive may
  surface as an archive-fetch failure instead. In that case the client
  SHOULD fetch `versions.json` to check the reason for removal and
  present more actionable information to the user.

The protocol is designed so that each client operation fetches only what
it needs:

| Operation | Needs from the index                                |
| --------- | --------------------------------------------------- |
| **solve** | `versions.json`                                     |
| **lock**  | `versions.json` + `.project.json` + `.meta.json`    |
| **sync**  | `project.kpar` (starting from an existing lockfile) |

`sync` does not re-read `versions.json` or the per-version JSON files, as
per-version files are immutable and the lockfile already records everything
`sync` needs to download the archive and verify it.

## 10. Digests and canonicalization

### Wire format

Advertised digests (`project_digest`, `kpar_digest` in `versions.json`)
MUST use the form `sha256:<64 lowercase hex>`. Uppercase hex is invalid
on the wire.

### `project_digest`

`project_digest` is SHA-256 over the canonical form of the `(info, meta)`
pair served at the same version directory. Because the `meta.checksum`
rule below constrains `.meta.json` to carry SHA-256 per-file digests
directly, the canonical form of conforming index content can be
computed from `.project.json` and `.meta.json` alone, without reading
`project.kpar` sources.

### `kpar_digest`

`kpar_digest` is SHA-256 over the raw bytes of `project.kpar`.

### `meta.checksum` values

Per-source-file checksums inside `.meta.json` (`meta.checksum`) are
`{ "value", "algorithm" }` pairs. In v0, `algorithm` MUST be `SHA256`
and `value` MUST consist of lowercase hex digits.

## 11. Server obligations

A conforming sysand index server MUST uphold:

- **`index.json` consistency.** Every project listed in `index.json` has a
  `versions.json` retrievable at its project directory ([§5]).
- **`versions.json` consistency.** The fields advertised in a `versions.json`
  entry agree with actual `.project.json`, `.meta.json`, and `project.kpar`
  files served at that version's directory.
- **File presence.** Every version listed in `versions.json` with
  `status` other than `removed` has all three per-version files
  available for retrieval.
- **Byte immutability.** Existing per-version files never have their
  bytes changed in place; a published `project.kpar` is either served
  with the same bytes forever or withdrawn (see retirement, below).
- **Retirement via `status`.** `versions.json` entries are retained:
  once an entry exists it is never removed, and its
  `version`, `usage`, `project_digest`, `kpar_size`, and `kpar_digest`
  fields never change. The only mutable field on an existing entry is
  `status` ([§8]). Permitted transitions are `available → yanked`,
  `available → removed`, and `yanked → removed`; no other transitions
  are permitted in v0 (in particular, no un-yank). `removed` is
  terminal, and moving a version to `removed` requires withdrawing
  its per-version files ([§9]).
- **Well-formed archives.** The full set of criteria for a well-formed
  archive is not frozen in v0 and is expected to evolve alongside the
  `sysand index` CLI (see [§15]).

## 12. Client obligations

A conforming sysand index client:

- Follows HTTP redirects on every index resource.
- MUST verify the streamed body of `project.kpar` against the advertised
  `kpar_digest` during download. In case of mismatch the archive MUST NOT
  be used.
- When it fetches either `.project.json` or `.meta.json`, MUST fetch both
  and MUST verify that their canonical `(info, meta)` digest equals the
  advertised `project_digest` before using either. A mismatch is a hard
  error.
- MUST NOT select a version whose [§8] `status` is not `"available"`
  for a new resolution. This applies to operations that consult
  `versions.json` to pick a version (solve, lock); `sync` replays a
  lockfile's pinned `(iri, version)` without re-solving and is
  unaffected by `status`.

## 13. Immutability and lockfile reproducibility

Byte immutability and retained `versions.json` entries ([§11]) have
direct consequences for sysand lockfiles:

- The pair `(iri, version)` is a stable identifier for a specific set of
  bytes; a lockfile referencing it stays valid against a conforming
  index for as long as the entry's `status` is not `"removed"`
  ([§8]).
- The `project_digest` and `kpar_digest` values advertised in
  `versions.json` are captured in a lockfile to detect any changes
  when downloading it later.

Retirement ([§8] `status`) and the lockfile contract:

- A `yanked` entry is fully compatible with existing lockfiles, as the
  original files are still served. Only new resolutions are affected.
- A `removed` entry breaks `sync` for any lockfile that pins it, as the
  project files for that version are no longer present in the server.

## 14. Forward compatibility

- Unknown fields in any JSON document MUST be ignored by the clients. Clients
  MAY still choose to inform the user of such changes.
- Protocol version is not explicitly provided anywhere currently.
- Breaking changes to this protocol are expected before v1.

## 15. `sysand index` CLI (preview)

The `sysand index` command group produces and maintains a sysand index
tree: laying out files, generating digests, and keeping
`versions.json` consistent with the per-version artifacts. This is the
only supported path for creating and mutating an index tree.

[§1]: #1-scope
[§2]: #2-implementability
[§3]: #3-discovery-and-configuration
[§4]: #4-layout
[§5]: #5-iri--path-resolution
[§5.1]: #51-iri-canonicalization-for-the-_iri-hash-bucket
[§6]: #6-pkgsysand-canonicalization
[§7]: #7-indexjson
[§8]: #8-versionsjson
[§9]: #9-per-version-files
[§10]: #10-digests-and-canonicalization
[§11]: #11-server-obligations
[§12]: #12-client-obligations
[§13]: #13-immutability-and-lockfile-reproducibility
[§14]: #14-forward-compatibility
[§15]: #15-sysand-index-cli-preview
[rfc2119]: https://www.rfc-editor.org/rfc/rfc2119.html
[rfc3986-43]: https://www.rfc-editor.org/rfc/rfc3986.html#section-4.3
[rfc3986-reg-name]: https://www.rfc-editor.org/rfc/rfc3986.html#section-3.2.2
[rfc9110-423]: https://datatracker.ietf.org/doc/html/rfc9110#section-4.2.3
[semver]: https://semver.org/spec/v2.0.0.html
[fluent-uri-normalize]: https://docs.rs/fluent-uri/0.4.1/fluent_uri/struct.Iri.html#method.normalize
[whatwg-url-domain-to-ascii]: https://url.spec.whatwg.org/#concept-domain-to-ascii
[idna-crate]: https://docs.rs/idna/
