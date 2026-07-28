# Credential storage and authentication commands

> **Status: shipped.** This document records the design behind the
> implemented credential persistence and the `sysand auth` command
> family. Sections 3-10 and 14 are the normative reference; section 11
> summarizes how the work landed.

## 1. Goal

Let users store an index credential once and have `sysand` reuse it across
runs, on Windows, macOS, and Linux, without `sysand` owning any secret
storage or cryptography. Acquisition of the token (GitLab/GitHub PAT,
Sysand Index API token) is out of scope: `sysand` stores what the user
provides.

v1 is deliberately narrow: bearer tokens, `login` / `logout` / `status` /
`whoami`, with plain and URL-template index locations both supported as
targets. Basic auth, raw-pattern commands, and the P2 protocol change are
deferred (§10).

## 2. Architecture context

Request-time application of credentials lives in `core/src/auth.rs` (a
per-URL glob map of policies, unauth-first with 4xx escalation); publish
OIDC in `core/src/commands/publish.rs`. `SYSAND_CRED_*` env variables
remain the per-run path. The credential store supplies the missing
middle: persist once, retrieve on later runs, feed the glob-based auth
layer.

## 3. Model and constraints

Three surfaces, and one access rule:

| Surface    | Probe path                 | Creds required? |
| ---------- | -------------------------- | --------------- |
| Discovery  | `sysand-index-config.json` | sometimes       |
| Index root | `index.json`               | sometimes       |
| API root   | `v1/whoami`, `v1/upload`   | always          |

Constraints:

- **C1 - unified read access.** Discovery and index root share one auth
  status (public or private together), collapsing the space to a 2x2.
- **C2 - one credential per index.** `auth login` stores one credential
  per index, used for both the read leg and the API leg. Separate
  read/write tokens are not a v1 concept (a later `auth set`, §10).
- **P2 - API presence is read from discovery.** An index has an API iff
  its discovery document advertises `api_root` (the protocol-level
  requirement landed separately, §12), so `api_root` is `Some` exactly
  when an API is advertised.
- **P1 - public discovery: not required.** Under C2 the single credential
  reads discovery on a private index, so there is no bootstrap paradox.
  Discovery may be private.

The collapsed situation space:

| #   | Read surface | API     | Example                     | Creds used for  |
| --- | ------------ | ------- | --------------------------- | --------------- |
| S1  | public       | none    | public static index         | nothing         |
| S2  | private      | none    | private static index        | read            |
| S3  | public       | present | official sysand.com         | write (publish) |
| S4  | private      | present | fully private dynamic index | read + write    |

## 4. Command surface (v1)

Under a `sysand auth` namespace:

| Command                          | Role                                                                                                                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sysand auth login [index-url]`  | validated, index-keyed bearer credential (see §5); no URL = the default index                                                                                                         |
| `sysand auth logout [index-url]` | remove a stored credential; no URL = the default index (symmetric with `login`)                                                                                                       |
| `sysand auth status`             | list stored credentials (never secrets), backend, and `SYSAND_CRED_*` shadowing; marks the entries that apply to the default index                                                    |
| `sysand auth whoami [index-url]` | query-only live identity via `v1/whoami` (advertised `api_root` required), with runtime credential selection (env over stored, §7); names the source used; no URL = the default index |

- **Bearer only in v1.** The token is entered via a hidden prompt
  ("Enter token for `<index>`:", neutral wording since the credential may
  be a forge PAT) or `--token-stdin`, never an inline value flag
  (shell-history / `ps` leakage). Basic auth (`--username`) and
  raw-pattern `auth set` / `unset` are deferred (§10); request-time basic
  auth via `SYSAND_CRED_*` still works.
- **`auth whoami` semantics.** Query-only: it never writes the credential
  store (no refresh of cached identity fields). Its discovery fetch uses
  the regular runtime read policy, so it works against private indexes.
  Exit code 0 only when the API accepted the credential; rejected,
  unreachable, redirected, and rate-limited responses are distinct
  nonzero errors. No advertised `api_root` gets a clear "no API" error.
- **Output style.** The CLI's existing conventions: styled leading word
  right-aligned in the 12-column gutter, anstyle tokens, byte-identical
  plain text when piped or under `NO_COLOR`, timestamps without
  sub-second precision.
- **Non-interactive safety.** If stdin is not a TTY and `--token-stdin` was
  not given, `login` fails fast ("no terminal for prompt; pass the token
  with `--token-stdin`") instead of hanging or reading a pipe as a secret.
- **Default index.** A bare `auth` command resolves its target from the
  default-index chain: `SYSAND_DEFAULT_INDEX` (comma-delimited), then a
  `default = true` index in configuration, else the built-in
  `DEFAULT_INDEX_URL` (`https://sysand.com`). `sysand publish` has no
  such default (its `--index` is required). If the chain yields **more
  than one** default index, bare `login` errors and asks for an explicit
  URL. `login` always **echoes the resolved index** before prompting (and
  on the `--token-stdin` path), so a project-configured default cannot be
  targeted silently. `auth status` reuses the chain only to mark the
  entries that apply to the default index and **never hard-errors over
  it**: an ambiguous chain prints a note and marks nothing; an invalid
  default silently marks nothing.
- **HTTP(S) only.** `auth login` against a non-HTTP(S) location (a local
  path or `file://` URL) errors with "not an HTTP(S) index; nothing to
  authenticate to".
- **Glob derivation** (§8): automatic from the login-target index URL,
  with discovery-advertised `index_root` / `api_root` adding further
  globs. No manual `--pattern` in v1; the `SYSAND_CRED_*` env var is the
  escape hatch for unusual layouts until `--pattern` / `auth set` land
  (§10). A **templated URL as the login target itself** (a `{path}` /
  `{path_raw}` template, for example a GitLab repository-files URL) is
  supported: the storage key is the template text with its literal-prefix
  anchor normalized through `url::Url` serialization (the rest stays
  verbatim: raw text, not a parsed URL), and the primary glob anchors on
  that prefix per §8. A template with no safe anchor (at least
  `scheme://authority/`) is rejected with a pointer to `SYSAND_CRED_*`.

The index URL is normalized (trailing slash, scheme) before use as the
storage key and for glob derivation, so different spellings do not create
duplicate entries.

## 5. Validation

Login has exactly one behavior, with no opt-out flag: discovery is
fetched for glob scoping (with the authenticated retry described below),
every surface the index supports is probed, and the refusal rule decides.
A static index has only the read surface; a dynamic index adds the API.

There is deliberately no way to disable credential validation. Offline or
unreachable-index logins degrade gracefully through the refusal rule:
nothing exercises the credential, so the login stores as "stored, not
validated" with warnings, and no secret is transmitted. False refusals
are engineered out rather than opted out of: 429 and redirects are never
verdicts, and acceptance by any exercised surface wins. For an index that
genuinely misbehaves, `SYSAND_CRED_*` environment credentials remain the
escape hatch. Validation is not levelled per surface either: `v1/whoami`
checks only that a token is _accepted_ (identity, not capability, §6), so
a "read-only" level would add a choice without payoff.

**Probe mechanism.** Validation cannot reuse the runtime unauth-first
policy, which returns only the final response and cannot report whether a
surface actually _accepted_ the credential. Each surface is probed as an
**unauth baseline then a forced-auth retry**: a surface counts as
accepted/tested only when the baseline was a 4xx and the forced retry
succeeded, so a public surface (200 unauth, credential never sent) is
"not tested", not "accepted". The API surface (`v1/whoami`) is always
authenticated, so only the forced request is sent. Validation is
discovery-first: fetch discovery, probe `index_root/index.json`, and,
**only if discovery advertised an `api_root`**, probe
`api_root/v1/whoami`, so a static plain-URL index is never phantom-probed
for an API it does not have.

**Login's discovery fetch is itself unauth-baseline-then-forced** (only
the backend-absent path of §9 stays strictly unauthenticated, since its
secret is discarded). Any 4xx baseline answer except 429 triggers one
forced-bearer retry with the in-hand secret; a rate-limited baseline gets
no extra request and never sees the secret. The retry must fire on 404
too: a private GitLab answers 404, not 401, when auth is missing, and the
credentialed answer is what distinguishes a hidden discovery document
from an absent one. Forced-retry outcomes: a 200 with a valid document is
used exactly like a public discovery success; a 404 is the authoritative
"no document" answer and reconstructs the flat topology with no warning;
everything else (other 4xx, 429, a redirect, 5xx, or a network failure)
is not a verdict and falls back to the URL-derived glob with a warning,
where the read probe uses the URL-derived `index.json` location so a
fully private index still gets its read leg exercised. The forced
discovery fetch never counts toward the validated claim: the `index.json`
probe is the sole read verdict, and claim strings stay `read`/`api`.

**Probes do not follow redirects**: a redirect would mean the verdict
comes from a different URL than the surface nominally probed (and a
cross-host redirect strips the header, misreading "rejected"), so a
redirected probe counts as "not tested" with a warning naming the
redirect target.

User-visible wording uses one stem, **validated**: "validated (read)",
"stored, not validated", rather than mixing "verified"/"unvalidated"
families.

**Refusal rule.** Store if the credential is _accepted by any surface it
actually tested_, warning about any surface that rejected or was
unreachable. Refuse only when at least one exercised surface rejected the
credential and none accepted it. A single-surface refusal names the
endpoint and its answer ("the index rejected the token for `<index>`
(`v1/whoami` answered HTTP 401); nothing was stored"); the
surface-enumeration wording is kept for the two-surface case. When the
read surface alone rejected with a 404, the message hedges with "or no
index exists at this URL": a valid token against a URL without an
`index.json` answers the same way, so the credential must not be blamed
outright. A surface counts as "tested" only if the credential was
exercised (a public read surface proves nothing; a 429 is never a
verdict). If nothing exercised the credential, store as "stored, not
validated".

This self-adjusts across the situation space: on a private index where
read works but the API rejects, store with a warning (the token still
reads); on a public-read index (sysand.com), `v1/whoami` is the only real
test and a rejected token is refused, keeping the publish flow protected;
when every exercised surface rejects, refuse.

**Basic-auth indexes must not dead-end.** A user of a private basic-auth
index will naturally try `auth login` and be refused. The read probe sees
the server's `WWW-Authenticate: Basic` challenge, so the refusal message
must say so and route the user to the working path: "this index uses
username/password (HTTP basic) authentication; configure
`SYSAND_CRED_<X>_BASIC_USER` / `_BASIC_PASS` instead (see docs)".

Never print a bare "validated"; always scope the claim to the surfaces
that actually accepted the credential.

## 6. The `v1/whoami` endpoint

Endpoint on the index API (server side, the `sysand-index` Django app),
under `api_root`. Its purpose is credential validation and identity for
`auth status`.

- `GET api_root/v1/whoami`, bearer credential. The server routes it under
  `api/` (`api/v1/whoami`); `api_root` carries the `/api/` segment, so the
  client's `api_root/v1/whoami` join is consistent with `v1/upload`.
- `200` on a valid, unexpired token; `401` otherwise. Under
  validation (§5) a `200` passes the API leg. The `401` body is
  unspecified (the client only reads the status).
- Body on `200`:

```json
{
  "subject": { "type": "user", "name": "alice" },
  "token": {
    "name": "laptop",
    "prefix": "sysand_u_1a2b3c4d",
    "expires_at": "2026-09-01T00:00:00Z"
  }
}
```

- `subject.type` is `user`, `project`, or `oidc`, so the endpoint is
  principal-agnostic (hence `whoami`, not `user`). `subject.name` is the
  **username** for a user token, the **project id** (`publisher/name`)
  for a project token, and the **publisher identity** for an OIDC token;
  distinct from `token.name` (the user-given token label).
- `token.expires_at` is always **returned** by whoami; the stored record
  persists it only when a validating login ran, hence
  "expires_at-if-known" in §8/§9. `token.prefix` is the non-secret
  display prefix (type prefix + first 8 hex).
- No `can_publish` flag and no scope list: at login there is no target
  project and every valid token can publish somewhere, so a capability
  boolean is vacuous; per-project authorization stays enforced at the
  upload (its existing `403`). An optional `?project=<id>` pre-flight may
  be added later.

## 7. Publish interaction

Publish is two legs with **different** credential handling:

- **Leg 1, discovery read** (`sysand-index-config.json`, to resolve
  `api_root`): the general read auth policy, unauth-first. `login` scopes
  the credential to cover the discovery/index root (§8), so a private
  index's discovery fetch gets it.
- **Leg 2, upload** (`POST api_root/v1/upload`): **bearer only**, sent
  proactively (an upload cannot be tried unauthenticated then retried).
  Publish checks env bearers first; only when **no env bearer matches**
  does it read the keyring blob (one keychain access). It selects the
  bearer whose glob matches the upload URL.

Publish's bearer selection uses **source precedence**: env bearer matches
first, then keyring, never one flat "exactly one match or error" over a
merged set. Within a source the exactly-one rule stands
(`AmbiguousPublishBearer` is per-source), with candidates carrying the
identical token collapsing to one match, like the reads rule in section 9.
Env and keyring stay **two maps** (source-tagged, with the selected
bearer's provenance threaded to the failure messages) with a two-stage
lookup, making the precedence real: a CI `SYSAND_CRED_*` overrides an
interactive login. The two-leg flow and trusted publishing are otherwise
unchanged.

- **Trusted-publishing precedence:** in `auto` mode publish uses OIDC
  trusted publishing when a supported CI environment is detected, and
  otherwise falls back to the bearer map (env > keyring). CI has no keyring,
  so the two rarely coexist.
- **Basic auth cannot publish** (leg 2 is bearer-only); a basic
  `SYSAND_CRED_*` entry is ignored for the upload.
- **No matching bearer** fails up front (before the upload) with a hint to
  run `sysand auth login <index>` to store a publish token.
- **Auth failures name the credential's source.** Because env shadows
  keyring, "re-run `sysand auth login`" is the wrong fix when the
  rejected bearer came from a stale `SYSAND_CRED_*` var (a fresh login
  would stay shadowed and the user would loop). So an upload auth failure
  states where the selected bearer came from and tailors the remediation
  (unset/rotate the env var vs re-login). A `403` (authorization, not
  authentication) additionally points at `sysand auth status`, which
  shows the stored `subject`, catching "this is a project token for a
  different project".
- **Fail fast on expiry:** if the selected bearer carries a known
  `expires_at` (§9) already past by more than a **generous clock-skew
  margin** (an hour; the stop is only an optimization, so it errs toward
  attempting rather than refuse a token the server would accept), publish
  stops before uploading and points at `sysand auth login`. The server's
  `401` remains the real authority.

Target-defaulting principle: irreversible remote effects require an
explicit target (`sysand publish --index` stays required), while
reversible local effects (an `auth login` writing a record that `auth
logout` removes) may fall back to the default index.

## 8. Glob scoping and conflict resolution

- **Source precedence, single match within a source.** For a given URL,
  all `SYSAND_CRED_*` (env) matches take precedence over all keyring
  matches. Within one source, the single-match rule applies (publish
  errors on a within-source ambiguity, identical-token matches collapsing
  first; reads try-all). v1 deliberately does **not** add longest-prefix
  tie-breaking; that is only needed once raw-pattern `auth set` or
  same-host nested logins create within-source overlaps (§10).
- **Glob coverage.** `login` anchors the primary glob on the **discovery URL
  the user supplied** (so the discovery fetch itself is authenticated), and
  additionally covers the resolved `index_root` and `api_root` when they
  diverge from it, minimal and non-overlapping. Templated URLs are anchored
  before `{path}` / `{path_raw}`.
- **Glob derivation is escaped and pinned.** URLs can legally contain
  glob metacharacters (an IPv6 literal reads as a globset character
  class; `{path}` templates as alternation), and `GlobMapBuilder` uses
  `literal_separator(true)`. So the derived glob is
  `globset::escape(<normalized root>)` + `**`, with the root normalized
  to end in `/` (for example `https://example.com/idx/**`), and both
  derivation and runtime matching use the same serialization
  (`url::Url::as_str()`) so IDN/percent-encoding agree on both sides. Two
  refinements: a templated root's anchor is the literal prefix cut back
  to its **last `/`**, and the root is skipped with a notice unless that
  anchor still parses as `scheme://authority/` or deeper (template
  parsing already rejects placeholders outside the path or query, so the
  reachable degenerate case is a template with no `/` after the
  authority, where the cut would land inside `https://`; the parse check
  is defense-in-depth for the rest); and within
  one login's derived set (the login URL plus its resolved `index_root`
  and `api_root`, never other stored records), a newly derived root that
  **subsumes** an already-derived one replaces it, keeping the set
  minimal and non-overlapping. Normative test requirements: the
  discovery-document URL, the `index.json` URL, and the upload URL
  (if present) each match the compiled derived set, and an IPv6-literal
  login (`https://[::1]:8000/`) works.
- **Divergent `api_root` (Case B).** If `api_root` nests under the derived
  root (Case A), one glob suffices. If it is a disjoint host/path, store the
  same credential under both globs (minimal, non-overlapping), so the upload
  URL matches exactly the api glob. Templated indexes are inherently Case B
  (their `api_root` is a disjoint plain URL).
- Each login is one record inside the single keyring blob (§9):
  `{key, globs, scheme, secret, expires_at-if-known}`, plus optional
  whoami-derived identity fields (`subject`, `token_name`,
  `token_prefix`) persisted by a validating login, plus `validated`: the
  surfaces that exercised and accepted the credential, serialized as
  `["read","api"]` strings and absent for nothing-exercised logins. So
  `logout` removes one record and `status` shows one login covering N
  patterns.
- **Discovery changes over time (globs are a login-time boundary).** Reads
  and publish re-fetch discovery live each run, but the stored globs are
  the login-time snapshot and are **not** auto-updated from discovery:
  auto-following a changed `api_root`/`index_root` would let a changed
  discovery silently redirect the stored token to a new host. When a
  discovery change moves a root **outside** the login's globs, the
  credential stops matching and the request fails cleanly; the generic
  "no bearer / re-run login" hint applies, and re-login re-derives the
  globs and re-validates.
  **Caveats, two ways the boundary is narrower than it sounds:**
  (a) it covers the login's own globs only; a broad `SYSAND_CRED_*` env
  pattern that also matches the moved root can still shadow it. And (b)
  **same-host redirects bypass the glob**: it is evaluated against the
  initial URL only, and the credential is forwarded to a same-host
  redirect target (existing `RestrictAuthentication`/reqwest behavior);
  cross-host redirects strip the header. The precise guarantee is
  therefore: "sysand does not itself auto-follow discovery to a
  **different host**"; within a host the server can move the credential
  via redirects.

**Trust model.** The discovery document at the URL you supply is the trust
anchor: `sysand` sends the credential to the `index_root`/`api_root` it
advertises (including a different host) and to `v1/whoami`, with no
same-origin or HTTPS restriction. Trusting the discovery URL means
trusting what it points at. Note the amplification honestly: over plain
`http`, a _one-time_ MITM at login can rewrite discovery to a hostile
`api_root`, which both leaks the freshly entered token and gets
**persisted** as a glob, so it keeps being sent there until re-login.
`http` (localhost or a trusted LAN) is still supported; the full
transport-security guidance lives in the docs (§13).

## 9. Storage, consumption, precedence

- **Backends:** OS keyring by default (macOS Keychain, Windows Credential
  Manager, Linux Secret Service via the `keyring` crate), with environment
  variables as the automatic fallback where no keyring exists. **No
  plaintext credentials file, ever.**
- **Single keyring entry.** All persisted credentials live in **one**
  keyring entry (`service = "sysand"`, `account = "credentials"`) holding
  a JSON blob of records (§8). Deliberate over a manifest file: the
  `keyring` crate cannot portably enumerate entries, and one blob is
  **atomic** (no metadata/secret drift), needs **no file**, and prompts
  the keychain at most once. `login` / `logout` read-modify-write the
  blob; `status` reads it. Removing the last record **deletes the keyring
  entry**, preserving the cheap no-entry path.
  - **Windows size limit.** Windows caps a blob at ~2.5 KB
    (`CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560), measured in **UTF-16 code
    units**, so ~1280 ASCII characters of serialized JSON; a single large
    JWT can exceed it on the first login. One message covers both cases
    ("credential store full on this platform (Windows ~2.5 KB limit);
    remove an unused login or use a smaller token"), and `status`/the
    error flag stale or expired entries so the user knows what to drop.
- **Blob format robustness.** The blob carries a `version` field that
  **gates the format**: a reader that does not recognize the version fails
  closed rather than guessing, reserving a clean migration point. Readers
  **round-trip** unknown record fields (serde-flatten passthrough maps),
  so an older binary's read-modify-write preserves a newer binary's
  fields. There is no record-level merge: concurrent writers are
  serialized by the file lock (below), last writer wins, and the field
  round-tripping is what keeps a concurrent newer binary's additions from
  being lost. On a parse failure, read-modify-write **fails closed**
  ("credential store unreadable; remove the `sysand` keyring entry to
  reset"), never silently treats the blob as empty, which would clobber
  all stored credentials on the next `login`.
- **Concurrency.** Read-modify-write is guarded by an **exclusive
  cross-process advisory OS file lock** (`flock`/`LockFileEx`, via the
  stable `std::fs::File` locking API), never an existence-based lock file
  (those go stale after a crash); read-only operations take its shared
  counterpart, both with a bounded wait and a clear error. The lock file
  lives in a **per-user** location with a stable order: the state dir
  (`XDG_STATE_HOME` on Linux), then the local data dir (`%LOCALAPPDATA%`
  on Windows), falling back to a dotdir in the home directory;
  deliberately never the session-scoped `XDG_RUNTIME_DIR`, which is often
  unset for cron, systemd, and container processes, so two same-user
  processes could lock different files while writing the same keyring
  entry. It is mode `0600`, never a world-writable shared path (lock
  squatting / symlink games); a lock file is not a credentials file, so
  the no-plaintext rule is untouched. The lock coordinates `sysand`
  processes with each other only, not against another application writing
  the same keyring entry.
- **Consumption and keyring access.** The blob is read only when a
  credential might actually be needed: the eager env layer
  (`RestrictAuthentication` from `SYSAND_CRED_*`, no keychain) runs
  first, and a lazy keyring layer is consulted only in the 4xx-escalation
  branch, so the blob read happens exactly when needed and
  env-before-keyring falls out for free. It can **not** be a
  `RestrictAuthentication` with a lazy inner map (that classifies the URL
  up front and would force the read). The composed policy is the
  dedicated `CredentialStoreAuthentication` combinator (CLI alias
  `CliAuthPolicy`); the lazy layer holds a cache and is not `Clone`, so
  publish's bearer extraction (`publish_bearer_auth_map`) is **by-ref**
  and clones the secrets it extracts, accepted as the cost. Escalation
  semantics: an env 4xx escalates into the keyring layer; a matching
  keyring record sends **forced** auth (not another unauth-first inner
  sequence, which would triple requests). **No-match must not
  re-request:** 404 is routine on the resolve path, and stock
  `SequenceAuthentication`, whose lower arm cannot see the higher arm's
  response, would re-issue an identical request on every ordinary 404,
  permanently doubling round-trips for logged-in users; the keyring layer
  therefore **passes the initial response down**, returning it untouched
  when no record matches. The blob cache serves the **request path
  only**, through one async accessor (`OnceCell` + `spawn_blocking`; the
  keyring crate is synchronous and a locked store can block for seconds);
  publish runs outside the async runtime and performs its own single
  direct store read, outside the cache. Several matching records:
  identical tokens **collapse to one retry**; genuinely distinct tokens
  warn and are tried in order (the reads try-all rule, section 8).
  - **Never read** for local/offline commands, for reads that succeed
    unauthenticated, or for users who never ran `auth login` (no entry: a
    cheap "not found", no unlock).
  - **Read once, then cache** on the first auth-relevant 4xx, on
    publish's upload leg, and on the `auth` commands. At most one
    keychain touch per command.
  - Reads escalate on **any** 4xx (not just 401/403), because some hosts
    (GitLab) answer `404` on missing/under-scoped auth. Cost: a
    logged-in user on a _locked_ Linux keyring may see one unlock prompt
    on a non-auth 404, rare, preferred over breaking the zero-config
    GitLab flow.
  - In steady state keychain reads are silent (Windows no prompt, macOS
    one-time "always allow", unlocked Linux no re-prompt).
- **Keyring error taxonomy:** _absent_ backend falls back to env;
  _present-but-locked/denied_ surfaces the error, suggests unlocking, and
  also names the `SYSAND_CRED_*` fallback, since on a headless box over SSH
  there is often no practical way to unlock the keyring.
- **No-keyring host:** `auth login` refuses to persist and prints the
  `SYSAND_CRED_*` lines to set, with the pattern value exact but the
  secret as a **`<token>` placeholder, never the entered value**:
  no-keyring hosts are typically CI/headless where stdout lands in
  captured job logs. Honest posture note: on such hosts the env fallback
  means the secret lives in same-user process environments and typically
  at rest in CI secret config or shell rc files; that is the accepted
  floor there, stated in the docs.
- **Precedence:** `SYSAND_CRED_*` > keyring > unauthenticated (source
  precedence, §8), so CI can override an interactive login. There is no
  separate runtime shadow warning: `auth status` shows per-entry shadowing,
  and the source-named auth-failure messages (§7) identify a stale env var
  exactly when it bites.
- **Expiry:** reactive first: when a request that exercised a stored
  credential ends in **any 4xx** (not just 401, GitLab-style hosts answer
  404 on bad auth) and the record's `expires_at` is past, print
  "credential for `<index>` may be expired or revoked; re-authenticate to
  store a fresh credential". Emitted in the library, so it names no CLI
  command. Proactive when known: a stored `expires_at` lets `auth status`
  show "expires in N days / expired".
- **`auth status` output:** one unified view of **everything sysand will
  authenticate with**, both sources, each entry tagged `Stored` or `Env`.
  Per stored entry: the key in the exact form `sysand auth logout <key>`
  accepts, the login-time validation claim (`validated (read)` /
  `validated (read, api)`, or a warn-styled `not validated`, the
  security-relevant case), covered globs, `subject` and token `prefix`
  (if a validating login ran), `expires_at` if stored, and whether a
  `SYSAND_CRED_*` var shadows it, never the secret. Entries of both
  sources that **apply to the default index** carry a dim
  `(default index)` marker: a stored entry on normalized-key equality or
  when one of its globs matches the default index root URL (sharing
  `shadowed by`'s root-URL approximation), an env entry when its pattern
  matches that root. Patterns compile leniently here, like shadow
  detection: one bad pattern never breaks status, and default-index
  resolution for the marker never errors (section 4). A source with
  nothing to show is omitted rather than announced with a negative; only
  when neither source has anything does status print "No credentials
  configured (no stored credentials, no `SYSAND_CRED_*` variables)." The
  no-usable-keyring note always prints when the backend is unusable (it
  is information, not a negative). No `scheme` column in v1 (always
  bearer for stored; env entries may be basic).
- **Re-login:** `auth login` over an existing entry for the same key
  overwrites it, printing "Replacing existing credential for `<index>`"
  (informational, suppressed under `--quiet`) before the write; the
  previous token is discarded locally (not revoked server-side).

## 10. Scope boundaries

**Deferred to later phases (intended, not v1):**

- `auth set` / `auth unset` (raw-pattern credentials) and the `--pattern`
  override on `login`.
- Basic auth via `--username` (request-time basic via `SYSAND_CRED_*` still
  works).
- Longest-prefix most-specific-glob-wins (needed only once `set` / nested
  logins create within-source overlaps).
  **Out of scope entirely:** acquisition beyond store-what-you-paste (OAuth
  apps, device flows, refresh-token lifecycle); a self-written encrypted vault
  or plaintext credentials file; a user-facing credential "label" concept;
  multi-account-per-host switching; git credentials (git keeps its own).

## 11. Phases (landed)

The work shipped in phases: the credential store and lazy auth policy
first (a behavior-neutral refactor to soak), then the server-side
`v1/whoami`, then the `auth` commands, then docs. **The whoami-before-login
ordering was load-bearing:** had `login` shipped first, a validating login
against the official index would probe whoami, get a 404, count the API
surface as rejected with the read surface "not tested", and refuse a
valid token.

CI notes that remain true: the workspace test lane runs `sysand-core` and
`sysand` together, and the CLI crate enables `sysand-core/keyring`, so
with resolver 3 feature unification the keyring-gated tests run in the
ordinary test lane (no separate keyring lane); prek's rust hooks are
pinned via `language_version`, since prek does not read
`rust-toolchain.toml`.

## 12. Protocol/spec changes (this repo, `design/`)

`design/index-api-protocol.md` specifies `v1/whoami` (§6), routed under
`api/`. The `design/index-protocol.md` require-`api_root` change landed
separately (an index has an API iff discovery advertises `api_root`; the
plain-URL default was dropped, a breaking change for third-party plain-URL
dynamic indexes, which must now serve `sysand-index-config.json` with an
explicit `api_root`).

## 13. Documentation (docs.sysand.com, in the `sysand-index` repo)

User docs live under `docs/source/` in the `sysand-index` repo and follow
that repo's `docs/README.md` authoring guidance: the authentication
reference and how-to, the `auth` command reference pages, and the
explanation page cover the `sysand auth` model, precedence, surfaces, and
trust model. CLI `about`/`long_about` text lives in this repo and the
reference pages mirror it.

## 14. Code placement (core vs CLI vs bindings)

The workspace splits into `core` (`sysand-core`, consumed by the CLI **and**
the py/js/java bindings; the js binding compiles to wasm) and `sysand` (the
CLI). Placement follows the repo's existing pattern: `do_*` command logic in
`core/src/commands/`, thin wrappers in `sysand/src/commands/`, optional
capabilities behind cargo features (`filesystem`, `networking`).

- **core, unconditional:** the record types, blob JSON codec (pure
  serde), and the store type itself (`LockedBlobStore`, including the
  cross-process file lock), generic over the `BlobBackend` storage seam.
- **core, behind the existing `networking` feature:** the lazy
  credential-store policy layer and its composition with the env layer,
  since `auth.rs` (and the `HTTPAuthentication` trait, defined over
  reqwest types) is already gated on `networking`. The wasm build is
  green because the js binding enables neither `networking` nor
  `filesystem`; keeping the policy layer under `networking` preserves
  that.
- **core, behind the `keyring` cargo feature** (not in `default`): the
  OS-keyring `BlobBackend` and the default lock-file path (uses `dirs`,
  mirroring `filesystem`). Enabled by the CLI and the py/java bindings;
  **off for the js/wasm binding**, the `keyring` crate does not build for
  wasm. A browser-side store could later implement the same backend
  trait, but that is not v1 and localStorage is not secure storage.
  - **Pure-Rust Secret Service.** keyring 4.x's `v1` feature uses the
    zbus Secret Service backend on glibc Linux, with no system libdbus
    and a `crypto-rust` session that keeps openssl out of the tree, so CI
    and contributor machines need no system libraries.
  - **musl exclusion.** musl targets are containers/CI where a Secret
    Service never runs, so the keyring dependency is target-gated off musl
    entirely and `OsKeyringBackend` has a musl stub returning the
    backend-absent error; the documented `SYSAND_CRED_*` fallback is the
    behavior there.
  - **Test seam.** CLI integration tests must never touch the real OS
    keyring (macOS prompts on differently-signed test binaries; CI has
    no Secret Service). The debug-build-only
    `SYSAND_TEST_CREDENTIAL_STORE` env var selects a file-backed or
    absent blob backend, refused loudly in release builds; every auth
    command and the lazy policy route through it. The shared CLI test
    harness defaults the seam to the absent backend, so a test reaches a
    real keyring only by opting out explicitly.
- **core `commands/auth.rs`, gated `all(filesystem, networking)`** (the
  same gate as publish): the `do_auth_*` orchestration (discovery fetch,
  glob derivation, validation probes, refusal rule), generic over the
  store's blob backend. **A library call must never prompt**: the secret
  arrives as a parameter, and there is no validation knob to plumb (§5).
- **sysand CLI:** the clap surface, hidden prompt, TTY detection,
  `--token-stdin`, default-index resolution, and user-facing messages; it
  constructs the keyring store and the composed policy and passes them
  into core.

**Index reading and the keyring.** Reads in core stay generic over
`HTTPAuthentication` (`IndexEnvironmentAsync<Policy>`), so core's
index-reading code gains no keyring knowledge at compile time. At runtime
the keyring is touched only inside the policy instance the host constructed
and handed in; on wasm no such instance exists and reads behave as today.
