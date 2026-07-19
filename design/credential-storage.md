# Credential storage and authentication commands (plan)

> **Status: plan / draft.** Not yet a stable contract. This document
> captures the agreed design for persisting index credentials and the
> `sysand auth` command family, so implementation can proceed in phases.

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

## 2. Current state

- **Request-time application of credentials already exists** in
  `core/src/auth.rs`: a per-URL glob map of auth policies (bearer, basic),
  combinators that try unauthenticated first and send the secret only on a
  4xx, and same-host redirect handling. The policy is built **eagerly and
  immutably** at startup in `sysand/src/lib.rs` from `SYSAND_CRED_*`.
- **Publish OIDC (trusted publishing)** exists in
  `core/src/commands/publish.rs` for CI.
- **Token sourcing is a stopgap:** `sysand/src/lib.rs` scans
  `SYSAND_CRED_*` environment variables on every run. There is no
  persistence: a user re-supplies env vars each invocation.

The gap this plan fills is the missing middle: persist a credential once,
retrieve it on later runs, feed it into the glob-based auth layer.

## 3. Model and constraints

Three surfaces, and one access rule:

| Surface    | Probe path                 | Creds required? |
| ---------- | -------------------------- | --------------- |
| Discovery  | `sysand-index-config.json` | sometimes       |
| Index root | `index.json`               | sometimes       |
| API root   | `v1/whoami`, `v1/upload`   | always          |

Constraints:

- **C1 - unified read access.** Discovery and index root share one auth
  status (public or private together). This removes pathological split
  permutations and collapses the space to a 2x2.
- **C2 - one credential per index.** `auth login` stores one credential per
  index, used for both the read leg and the API leg. Separate read/write
  tokens are not a v1 concept (a later `auth set` could cover them, §10).
- **P2 - API presence is read from discovery (consumed, not enforced
  here).** This plan treats an index as having an API iff its discovery
  document advertises `api_root`, and derives globs accordingly. It does
  **not** change the runtime default (today a plain-URL index still defaults
  `api_root` to its root). Enforcing "`api_root` required" at the protocol
  level, and dropping that default, is a **separate, decoupled change**
  (§12), out of this plan's phases.
- **P1 - public discovery: not required.** Under C2 the single credential
  reads discovery on a private index, so there is no bootstrap paradox.
  Current behavior (discovery may be private) stands.

The collapsed situation space:

| #   | Read surface | API     | Example                     | Creds used for  |
| --- | ------------ | ------- | --------------------------- | --------------- |
| S1  | public       | none    | public static index         | nothing         |
| S2  | private      | none    | private static index        | read            |
| S3  | public       | present | official sysand.com         | write (publish) |
| S4  | private      | present | fully private dynamic index | read + write    |

## 4. Command surface (v1)

Under a `sysand auth` namespace:

| Command                          | Role                                                                            |
| -------------------------------- | ------------------------------------------------------------------------------- |
| `sysand auth login [index-url]`  | validated, index-keyed bearer credential (see §5); no URL = the default index   |
| `sysand auth logout [index-url]` | remove an index login; no URL = the default index (symmetric with `login`)      |
| `sysand auth status`             | list stored credentials (never secrets), backend, and `SYSAND_CRED_*` shadowing |

- **Bearer only in v1.** The token is entered via a hidden prompt
  ("Enter token for `<index>`:", neutral wording since the credential may be
  a forge PAT rather than a Sysand API token) or `--token-stdin`, never an
  inline value flag (shell-history / `ps` leakage). Basic auth
  (`--username`) and raw-pattern `auth set` / `unset` are deferred (§10);
  request-time basic auth via `SYSAND_CRED_*` still works.
- **`auth whoami` semantics.** Query-only: it never writes the credential
  store (no refresh of cached identity fields). Its discovery fetch uses
  the regular runtime read policy (env credentials and stored logins
  apply), so it works against private indexes. Exit code 0 only when the
  API accepted the credential; rejected, unreachable, redirected, and
  rate-limited responses are distinct nonzero errors. An index whose
  discovery did not advertise an `api_root` gets a clear "no API" error.
- **Output style.** All auth command output follows the CLI's existing
  conventions: the styled leading word right-aligned in the 12-column
  gutter (as in `Publishing ...` / `Published ...`), anstyle tokens for
  emphasis, and byte-identical plain text when piped or under `NO_COLOR`.
  Timestamps are shown without sub-second precision.
- **Non-interactive safety.** If stdin is not a TTY and `--token-stdin` was
  not given, `login` fails fast ("no terminal for prompt; pass the token
  with `--token-stdin`") instead of hanging or reading a pipe as a secret.
- **Default index.** `sysand auth login` with no URL resolves the target
  from the default-index chain: `--default-index` / `SYSAND_DEFAULT_INDEX`,
  then a `default = true` index in configuration, else the built-in
  `DEFAULT_INDEX_URL` (`https://sysand.com`). Note `sysand publish` has no
  such default (its `--index` is required), so this chain is defined here,
  not borrowed. If the chain yields **more than one** default index, bare
  `login` errors and asks for an explicit URL. `login` always **echoes the
  resolved index** before prompting (and on the `--token-stdin` path), so a
  project-configured default cannot be targeted silently.
- **HTTP(S) only.** `auth login` against a non-HTTP(S) location (for
  example a local file path, which index resolution accepts elsewhere)
  errors with "not an HTTP(S) index; nothing to authenticate to".
- **Glob derivation** (§8): automatic from the URL; no manual `--pattern` in
  v1. If derivation is ever wrong for an unusual layout, the `SYSAND_CRED_*`
  env var is the escape hatch until `--pattern` / `auth set` land (§10).
  v1 implementation note: a **templated URL as the login target itself**
  (the user typing a `{path}` template into `auth login`/`logout`) is
  rejected with a pointer to `SYSAND_CRED_*`; a templated `index_root`
  advertised **by discovery** is anchored per §8 as planned. Revisit if
  template-target logins turn out to matter.

The index URL is normalized (trailing slash, scheme) before use as the
storage key and for glob derivation, so different spellings do not create
duplicate entries.

## 5. Validation

`auth login` takes `--validation true|false` (default `true`). It maps to
an `Option<bool>` argument (absent = `None` = the default), so the language
bindings expose a clean optional keyword: `validation: Optional[bool] =
None`. This intentionally diverges from the repo's `--no-<flag>` boolean
convention (for example `--no-lock`), which binds as a required,
negative-sense `no_lock: bool`; a positive `Option<bool>` reads better as an
optional keyword across the py/js/java bindings.

- `--validation true` (default): probe every surface the index supports and
  store unless the credential is rejected everywhere it was actually tested
  (see the refusal rule below). A static index has only the read surface; a
  dynamic index adds the API.
- `--validation false`: store without any credential probe. Discovery is
  still fetched best-effort for glob scoping (§8); if unreachable, fall back
  to the URL-derived glob with a warning. Use it offline, or when a probe
  would false-refuse.

Validation is a boolean, not per-surface levels: since `v1/whoami` checks
only that a token is _accepted_ by the API (identity, not capability, §6),
validating everything almost never wrongly refuses a valid token, so a
"read-only" level would add a choice without payoff. `--validation` could
later give way to a levelled flag without disrupting this default.

**Probe mechanism.** Validation cannot reuse the runtime unauth-first
policy, which returns only the final response and cannot report whether a
surface actually _accepted_ the credential. Each surface is probed as an
**unauth baseline then a forced-auth retry**: a surface counts as
accepted/tested only when the unauth baseline was a 4xx and the forced retry
then succeeded, so a public surface (200 unauth, credential never sent) is
correctly "not tested", not "accepted". The API surface (`v1/whoami`) is
always authenticated, so its baseline is a known 401 and only the forced
request is needed. Validation is discovery-first (`api_root` is known only
after reading discovery): fetch discovery, resolve `index_root` and
`api_root`, probe `index_root/index.json`, and, **only if discovery
advertised an `api_root`** (not the runtime plain-URL default, §3), probe
`api_root/v1/whoami`, so a static plain-URL index is never phantom-probed
for an API it does not have. Advertised-vs-defaulted needs a flag on the
resolved endpoints (for example `api_root_advertised: bool`), true only in
the explicit-field arm of discovery parsing, false in both defaulting arms
and the no-document path. If the discovery fetch itself fails or is
rejected, the read probe falls back to the URL-derived `index.json`
location, so a private index whose discovery 401s still gets its read leg
exercised. **Probes do not follow redirects**: a redirect
would mean the verdict comes from a different URL than the surface nominally
probed (and a cross-host redirect strips the header, misreading "rejected"),
so a redirected probe counts as "not tested" with a warning naming the
redirect target.

User-visible wording uses one stem, **validated**: "validated (read)",
"stored, not validated", matching the `--validation` flag, rather than
mixing "verified"/"unvalidated" families.

**Refusal rule.** Store if the credential is _accepted by any surface it
actually tested_, warning about any surface that rejected or was
unreachable. Refuse only when at least one exercised surface rejected the
credential and none accepted it. A surface counts as "tested" only if the
credential was exercised: a _public_ read surface returns 200 without
sending the credential, so it proves nothing. A 429 response is never a
verdict; a rate-limited probe counts the surface as not tested. If nothing exercised the
credential (fully public read with no API, or every probe unreachable),
store as "stored, not validated".

This self-adjusts across the situation space:

- Private index, read works, API rejects: store with an "API access failed"
  warning (the token is still useful for reading).
- Public-read index (for example sysand.com): the read probe never tests
  the token, so `v1/whoami` is the only real test, and a rejected token is
  refused, keeping the publish flow protected.
- Every exercised surface rejects: refuse.

**Basic-auth indexes must not dead-end.** In bearer-only v1, a user of a
private basic-auth index will naturally try `auth login` and be refused. The
read probe sees the server's `WWW-Authenticate: Basic` challenge, so the
refusal message must say so and route the user to the working path: "this
index uses username/password (HTTP basic) authentication; configure
`SYSAND_CRED_<X>_BASIC_USER` / `_BASIC_PASS` instead (see docs)".

Never print a bare "validated"; always scope the claim to the surfaces that
actually accepted the credential.

## 6. The `v1/whoami` endpoint

New endpoint on the index API (server side, the `sysand-index` Django app),
under `api_root`. Its purpose is credential validation and identity for
`auth status`.

- `GET api_root/v1/whoami`, bearer credential. The server routes it under
  `api/` (`api/v1/whoami`); `api_root` carries the `/api/` segment, so the
  client's `api_root/v1/whoami` join is consistent with `v1/upload`.
- `200` on a valid, unexpired token; `401` otherwise. Under
  `--validation true` a `200` passes the API leg (§5). The `401` body is
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
  **username** for a user token, the **project id** (`publisher/name`) for a
  project token, and the **publisher identity** for an OIDC token; it is
  distinct from `token.name` (the user-given token label).
- `token.expires_at` is always **returned** by whoami (the model's
  `expires_at` is non-nullable); the stored record persists it only when a
  validating login actually ran, hence "expires_at-if-known" in §8/§9.
  `token.prefix` is the non-secret display prefix (type prefix + first 8
  hex).
- No `can_publish` flag and no scope list: at login there is no target
  project and every valid token can publish somewhere, so a capability
  boolean is vacuous; per-project authorization stays enforced at the upload
  (its existing `403`). An optional `?project=<id>` pre-flight may be added
  later.

## 7. Publish interaction

Publish is two legs with **different** credential handling:

- **Leg 1, discovery read** (`sysand-index-config.json`, to resolve
  `api_root`): uses the general read auth policy, unauth-first, authenticated
  only if the index is private. `login` scopes the credential to cover the
  discovery/index root (§8), so a private index's discovery fetch gets it.
- **Leg 2, upload** (`POST api_root/v1/upload`): **bearer only**, sent
  proactively (an upload cannot be tried unauthenticated then retried).
  Publish checks env bearers first; only when **no env bearer matches** does
  it read the keyring blob (one keychain access), consistent with "never
  read the keyring when env already works". It then selects the bearer whose
  glob matches the upload URL.

The one change to publish's bearer selection is **source precedence**: try
env bearer matches first (single match within env), then keyring (single
match within keyring), instead of one flat "exactly one match or error" over
the merged set. Within a source the existing exactly-one rule stands (its
`AmbiguousPublishBearer` error becomes per-source). Concretely:
`publish_bearer_auth_map` (the by-ref bearer extraction in
`core/src/auth.rs`) and `resolve_publish_bearer_from_config` (in
`core/src/commands/publish.rs`) keep env and keyring as **two maps**
(source-tagged, with the selected bearer's provenance threaded to the
failure messages) with a two-stage lookup, never collapsing to one flat
`GlobMap`. This makes the stated
precedence real, a CI `SYSAND_CRED_*` overrides an interactive login. The
two-leg flow and trusted publishing are otherwise unchanged.

- **Trusted-publishing precedence:** in `auto` mode publish uses OIDC
  trusted publishing when a supported CI environment is detected, and
  otherwise falls back to the bearer map (env > keyring). CI has no keyring,
  so the two rarely coexist.
- **Basic auth cannot publish** (leg 2 is bearer-only); a basic
  `SYSAND_CRED_*` entry is ignored for the upload.
- **No matching bearer** fails up front (before the upload) with a hint to
  run `sysand auth login <index>` to store a publish token.
- **Auth failures name the credential's source.** Because env shadows
  keyring, "re-run `sysand auth login`" is the wrong fix when the rejected
  bearer came from a stale `SYSAND_CRED_*` var (a fresh login would stay
  shadowed and the user would loop). So an upload auth failure states where
  the selected bearer came from ("from `SYSAND_CRED_TEAMIDX`" vs "from your
  stored login for `<index>`") and tailors the remediation (unset/rotate the
  env var vs re-login). A `403` (authorization, not authentication)
  additionally points at `sysand auth status`, which shows the stored
  `subject`, catching "this is a project token for a different project".
- **Fail fast on expiry:** if the selected bearer carries a known
  `expires_at` (§9) already past (with a small skew margin, since a fast
  client clock could false-trip), publish stops before uploading the archive
  and points at `sysand auth login`. The server's `401` remains the real
  authority; the escape hatches are re-login or an env var.

## 8. Glob scoping and conflict resolution

- **Source precedence, single match within a source.** For a given URL, all
  `SYSAND_CRED_*` (env) matches take precedence over all keyring matches
  (so CI can override an interactive login). Within one source, the existing
  single-match rule applies (publish errors on a within-source ambiguity;
  reads try-all). v1 deliberately does **not** add longest-prefix
  tie-breaking, that is only needed once raw-pattern `auth set` or
  same-host nested logins create within-source overlaps (§10).
- **Glob coverage.** `login` anchors the primary glob on the **discovery URL
  the user supplied** (so the discovery fetch itself is authenticated), and
  additionally covers the resolved `index_root` and `api_root` when they
  diverge from it, minimal and non-overlapping. Templated URLs are anchored
  before `{path}` / `{path_raw}`.
- **Glob derivation is escaped and pinned.** URLs can legally contain glob
  metacharacters (an IPv6 literal `https://[::1]:8000/` reads as a globset
  character class; `{path}` templates read as alternation), and
  `GlobMapBuilder` uses `literal_separator(true)` (`*` does not cross `/`).
  So the derived glob is `globset::escape(<normalized root>)` + `**`, with
  the root normalized to end in `/` (for example
  `https://example.com/idx/**`), and both derivation and runtime matching
  use the same serialization (`url::Url::as_str()`) so IDN/percent-encoding
  agree on both sides. Two refinements: a templated root's literal-prefix
  anchor must be **clamped to at least `scheme://authority/`** (a
  query-position placeholder would otherwise degenerate the anchor to
  `https://**`), skipping the root with a notice when even that is not
  meaningful; and when a newly derived root **subsumes** an already-derived
  one (string-prefix coverage with both sides slash-terminated), it
  replaces it, keeping the set minimal and non-overlapping. Normative test
  requirements: the discovery-document URL, the `index.json` URL, and the
  upload URL each match the compiled derived set, and an IPv6-literal login
  (`https://[::1]:8000/`) works.
- **Divergent `api_root` (Case B).** If `api_root` nests under the derived
  root (Case A), one glob suffices. If it is a disjoint host/path, store the
  same credential under both globs (minimal, non-overlapping), so the upload
  URL matches exactly the api glob. Templated indexes are inherently Case B
  (their `api_root` is a disjoint plain URL).
- Each login is one record (`{key, globs, scheme, secret,
expires_at-if-known}`, plus optional whoami-derived identity fields
  `subject`, `token_name`, `token_prefix` persisted by a validating login)
  inside the single keyring blob (§9), so `logout` removes it and `status`
  shows one login covering N patterns.
- **Discovery changes over time (globs are a login-time boundary).** Reads
  and publish re-fetch discovery live each run, but the stored globs are the
  login-time snapshot and are **not** auto-updated from discovery. This is
  deliberate: auto-following a changed `api_root`/`index_root` would let a
  changed (see the trust model below) discovery silently redirect the stored
  token to a new host. So when a discovery change moves a root **outside**
  the login's globs, the credential stops matching and the request fails
  cleanly rather than following, either way safe. **Best-effort diagnostic
  (may land later):** where sysand can correlate the failing request with a
  login whose snapshot globs no longer cover the resolved root, it prints
  "the index configuration has changed since you logged in; re-run
  `sysand auth login <index>` to update". This correlation is non-trivial on
  the read path (the auth layer sees per-request URLs, not the resolved
  index identity), so if it does not ship in the first cut the generic "no
  bearer / re-run login" hint applies. Re-login re-derives the globs and
  re-validates.
  **Caveats, two ways the boundary is narrower than it sounds:**
  (a) it covers the login's own globs only, a broad `SYSAND_CRED_*` env
  pattern that also matches the moved root can still shadow it (env is
  user-controlled and takes precedence); and (b) **same-host redirects
  bypass the glob**: the glob is evaluated against the initial URL only, and
  the credential is then forwarded to a same-host redirect target (existing
  `RestrictAuthentication`/reqwest behavior), so a server that answers a
  matched URL with `302` to another path on the same host receives the
  credential there without any glob re-check. Cross-host redirects strip the
  header. The precise guarantee is therefore: "sysand does not itself
  auto-follow discovery to a **different host**"; within a host the server
  can move the credential via redirects.

**Trust model.** The discovery document at the URL you supply is the trust
anchor: `sysand` sends the credential to the `index_root`/`api_root` it
advertises (including a different host) and to `v1/whoami`, with no
same-origin or HTTPS restriction. Trusting the discovery URL means trusting
what it points at. Note the amplification honestly: over plain `http`, a
_one-time_ MITM at login can rewrite discovery to a hostile `api_root`,
which both leaks the freshly entered token and gets **persisted** as a glob,
so it keeps being sent there until re-login, not merely a single
eavesdropped request. `http` (localhost or a trusted LAN) is still
supported; the full transport-security guidance lives in the docs (§13).

## 9. Storage, consumption, precedence

- **Backends:** OS keyring by default (macOS Keychain, Windows Credential
  Manager, Linux Secret Service via the `keyring` crate), with environment
  variables as the automatic fallback where no keyring exists. **No
  plaintext credentials file, ever.**
- **Single keyring entry.** All persisted credentials live in **one**
  keyring entry (for example `service = "sysand"`, `account =
"credentials"`) holding a JSON blob: a list of records `{key, globs,
scheme, secret, expires_at-if-known}` plus optional whoami-derived
  identity fields (`subject`, `token_name`, `token_prefix`) written by a
  validating login. Deliberate over a manifest file:
  the `keyring` crate cannot portably enumerate entries, and one blob is
  **atomic** (no metadata/secret drift), needs **no file**, and prompts the
  keychain at most once. `login` / `logout` read-modify-write the blob;
  `status` reads it. Removing the last record **deletes the keyring
  entry**, preserving the cheap no-entry path for users who logged out of
  everything.
  - **Windows size limit.** Windows caps a blob at ~2.5 KB
    (`CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560), measured in **UTF-16 code
    units**, so ~1280 ASCII characters of serialized JSON. With small
    tokens that is roughly ten entries; large JWTs, fewer, and a single
    token can exceed it on the first login. One message covers both:
    "credential store full on this platform (Windows ~2.5 KB limit);
    remove an unused login or use a smaller token", and `status`/the error
    flag stale or expired entries so the user knows what to drop.
- **Blob format robustness.** The blob carries a `version` field; readers
  do not merely tolerate unknown record fields but **round-trip** them
  (serde-flatten passthrough maps), so an older binary's read-modify-write
  preserves a newer binary's fields instead of dropping them. On a parse failure
  (interrupted write, older/newer sysand, another same-user process's
  scribble) read-modify-write **fails closed**, "credential store
  unreadable; remove the `sysand` keyring entry to reset", never silently
  treats the blob as empty, which would clobber all stored logins on the
  next `login`.
- **Concurrency.** Read-modify-write is guarded by a **cross-process
  advisory OS file lock** (`flock`/`LockFileEx`, via a crate like
  `fd-lock`), never an existence-based lock file (those go stale after a
  crash). The lock file lives in a **per-user** location
  (`XDG_RUNTIME_DIR`/`XDG_STATE_HOME` on Linux, `%LOCALAPPDATA%` on
  Windows, falling back to the home directory), mode `0600`, never a
  world-writable shared path (lock squatting / symlink games), with a
  bounded wait and a clear error. A lock file is not a credentials file, so
  the no-plaintext rule is untouched. Parallel `sysand` invocations are
  real; an in-process mutex alone would lose one writer's record.
- **Consumption and keyring access.** The blob is read only when a
  credential might actually be needed, to avoid unnecessary keychain prompts.
  This requires a credential source the auth policy consults on demand. The
  natural shape (implementable from the existing combinators) is
  `SequenceAuthentication<EnvLayer, LazyKeyringLayer>`: the env layer is the
  existing eager `RestrictAuthentication` from `SYSAND_CRED_*` (no keychain),
  and the lazy keyring layer is consulted only in `SequenceAuthentication`'s
  4xx-escalation branch, so the blob read happens exactly when needed and
  env-before-keyring falls out for free. Note it can **not** be a
  `RestrictAuthentication` with a lazy inner map (that classifies the URL up
  front and would force the read). The composed policy is a dedicated
  `CredentialStoreAuthentication` combinator with a concrete CLI alias
  (`CliAuthPolicy`) used by `command_publish`; it takes the place of an
  eager, immutable policy built once in `sysand/src/lib.rs`. Because the
  lazy layer holds a cache (`OnceCell`) it is not `Clone`, so publish's
  bearer extraction (`publish_bearer_auth_map`) is **by-ref** and clones
  the secrets it extracts, accepted as the cost of the lazy layer. It defers the blob read to
  the first auth-relevant 4xx (or publish / `auth` command), reads the whole
  blob once, and caches it for the process. Escalation semantics: a
  **failed** env credential (env 4xx) escalates into the keyring layer; a
  matching keyring record sends **forced** auth (not another unauth-first
  inner sequence, which would triple requests). **No-match must not
  re-request:** 404 is a routine outcome on the resolve path
  (`MissingPolicy::AllowNotFound`, version probing), so stock
  `SequenceAuthentication`, whose lower arm cannot see the higher arm's
  response, would re-issue an identical unauthenticated request on every
  ordinary 404, permanently doubling round-trips for logged-in users. The
  keyring layer therefore needs a **variant combinator that passes the
  initial response down**, returning it untouched when no record matches.
  Two further semantics to pin: the blob cache needs **dual accessors**,
  an async one (`OnceCell` + `spawn_blocking`, since the keyring crate is
  synchronous and a locked store can block for seconds) for the request
  path, and a plain sync one for publish, which runs outside the async
  runtime and must not nest `block_on`. When several records' globs match
  one URL: matches carrying the **identical token collapse to one retry**;
  genuinely distinct tokens warn and are tried in order (the reads
  try-all rule, section 8).
  - **Never read** for local/offline commands, for reads that succeed
    unauthenticated (public indexes return 200 and never touch the keyring),
    or for users who never ran `auth login` (no entry: a cheap "not found",
    no unlock).
  - **Read once, then cache** on the first auth-relevant 4xx, on publish's
    upload leg, and on the `auth` commands. At most one keychain touch per
    command.
  - Reads escalate on **any** 4xx (not just 401/403), because some hosts
    (GitLab) answer `404` on missing/under-scoped auth. Cost: a logged-in
    user on a _locked_ Linux keyring may see one unlock prompt on a non-auth
    404, rare, once per session, preferred over breaking the zero-config
    GitLab flow. (Future refinement, gated on `keyring` support for
    non-forcing reads: force-unlock only on `401`/`403`.)
  - In steady state keychain reads are silent (Windows no prompt, macOS
    one-time "always allow", unlocked Linux no re-prompt).
- **Keyring error taxonomy:** _absent_ backend falls back to env;
  _present-but-locked/denied_ surfaces the error, suggests unlocking, and
  also names the `SYSAND_CRED_*` fallback, since on a headless box over SSH
  there is often no practical way to unlock the keyring.
- **No-keyring host:** `auth login` refuses to persist and prints the
  `SYSAND_CRED_*` lines to set, with the pattern value exact but the secret
  as a **`<token>` placeholder, never the entered value**: no-keyring hosts
  are typically CI/headless where stdout lands in captured job logs, and
  echoing the secret would defeat the hidden prompt. Honest posture note:
  on such hosts the env fallback means the secret lives in same-user
  process environments and typically ends up at rest in CI secret config or
  shell rc files; that is the accepted floor there, stated in the docs.
- **Precedence:** `SYSAND_CRED_*` > keyring > unauthenticated (source
  precedence, §8), so CI can override an interactive login. There is no
  separate runtime shadow warning: `auth status` shows per-entry shadowing,
  and the source-named auth-failure messages (§7) identify a stale env var
  exactly when it bites.
- **Expiry:** reactive first, when a request that exercised a stored
  credential ends in **any 4xx** and the record's `expires_at` is past,
  print "credential for `<index>` may be expired or revoked; re-run
  `sysand auth login <index>`", any 4xx, not just 401, because
  GitLab-style hosts answer 404 on bad auth and the blob is already loaded
  on that path. Proactive when known, `expires_at` (stored from `v1/whoami`
  at login, absent for static/read-only or non-validated logins) lets
  `auth status` show "expires in N days / expired".
- **`auth status` output:** one unified view of **everything sysand will
  authenticate with**, both sources, each entry tagged `stored` or `env`.
  Per stored entry: the key printed in the exact form
  `sysand auth logout <key>` accepts, covered globs, `subject` and token
  `prefix` (from whoami, if a validating login ran), `expires_at` if
  stored, and whether a `SYSAND_CRED_*` var shadows it, never the secret.
  Env entries list the variable label and pattern. No `scheme` column in v1
  (always bearer for stored; env entries may be basic).
- **Re-login:** `auth login` over an existing entry for the same key
  overwrites it, printing "replacing existing credential for `<index>`"
  before the write; the previous stored token is discarded locally (not
  revoked server-side).

## 10. Scope boundaries

**Deferred to later phases (intended, not v1):**

- `auth set` / `auth unset` (raw-pattern credentials) and the `--pattern`
  override on `login`.
- Basic auth via `--username` (request-time basic via `SYSAND_CRED_*` still
  works).
- Longest-prefix most-specific-glob-wins (needed only once `set` / nested
  logins create within-source overlaps).
- **P2 enforcement** (dropping the plain-URL `api_root` default), a
  separate protocol change on its own timeline (§12); this plan only
  consumes `api_root` when advertised.

**Out of scope entirely:** acquisition beyond store-what-you-paste (OAuth
apps, device flows, refresh-token lifecycle); a self-written encrypted vault
or plaintext credentials file; a user-facing credential "label" concept;
multi-account-per-host switching; git credentials (git keeps its own).

## 11. Build phases

Each phase is independently shippable.

1. **Credential store.** Single-keyring-blob store (versioned format,
   fail-closed on parse errors) with cross-process advisory file locking +
   env fallback, keyring error taxonomy, index-URL normalization,
   source-precedence lookup, Windows size-limit handling. Introduce the
   **deferred/cached auth policy** that reads the blob on demand and caches
   it, replacing the eager `SYSAND_CRED_*`-only build in
   `sysand/src/lib.rs`. This phase's value is landing and soaking the risky
   refactor with no behavior change; user-visible persistence arrives with
   phase 3's `login` (until then only tests populate the store). Crate
   placement per §14.
2. **`v1/whoami`** (server side, `sysand-index` repo, **already
   implemented and merged there**): identity + token metadata, acceptance
   via HTTP status, routed at `api/v1/whoami`. **This ordering is
   load-bearing:** if `login` shipped first, a validating login against
   the official index (public read + advertised `api_root`) would probe
   whoami, get a 404, count the API surface as rejected with the read
   surface "not tested", and refuse a valid token. The cross-repo
   dependency (and the §13 docs work) is tracked in `sysand-index`'s
   `.agents/plans/`.
3. **`auth login` / `logout` / `status` / `whoami`.** Bearer-only;
   default-index resolution + echo; non-interactive fail-fast; discovery
   fetch; escaped glob derivation (discovery/index root + divergent
   `api_root`; templated targets anchored at their literal prefix);
   validation with `--no-validation` opt-out, unauth-baseline-then-forced
   probes (no-redirect), the refusal rule, and the basic-auth routing
   message; `expires_at` persistence; source-named auth-failure messages
   (§7); the query-only `whoami` command; gutter-aligned styled output
   (§4). The tailored discovery-drift message (§8) is **best-effort**
   here, not
   required for the phase to ship.
4. **Docs and specs** (§12, §13). Protocol specs in this repo; user docs in
   the `sysand-index` repo (docs.sysand.com).

Separate, not part of these phases: **P2 enforcement** (§10, §12) and the
deferred `auth set` / basic-auth work.

CI notes for these phases: the test lanes do not enable `-F keyring`, so
keyring-gated tests only compile via `clippy --all-features` until a
keyring lane is added; and the prek job needs its rust hooks pinned via
`language_version` (handled separately from this plan), since prek does
not read `rust-toolchain.toml`.

## 12. Protocol/spec changes (this repo, `design/`)

- `design/index-api-protocol.md`: specify `v1/whoami` (§6), routed under
  `api/`.
- `design/index-protocol.md` (**decoupled change**, not this plan's
  phases): enforce "an index has an API iff discovery advertises `api_root`"
  and drop the plain-URL default. This is a **breaking change**, a
  third-party plain-URL _dynamic_ index that relies on the defaulted
  `api_root` (no discovery document, or one without the field) becomes
  read-only and must serve `sysand-index-config.json` with an explicit
  `api_root` (the official index already does). Both the field-absent branch
  and the 404 `flat()` path in `core/src/env/discovery.rs` must change.

## 13. Documentation (docs.sysand.com, in the `sysand-index` repo)

Cross-repo: the published docs live under `docs/source/` in `sysand-index`,
not here. Follow that repo's `docs/README.md` (sentence case, no em-dash,
trailing-slash links). Pages to touch:

- **Reference, rewrite** `docs/source/client/reference/authentication.md`:
  the `sysand auth` model, single-keyring storage, `--validation`,
  precedence (`SYSAND_CRED_*` > keyring), read/API surfaces, the trust
  model. Keep the `SYSAND_CRED_*` reference, it remains the CI / no-keyring
  path (and the only basic-auth path in v1).
- **Reference, new** `docs/source/client/reference/commands/auth/`
  subdirectory (mirroring `commands/index/` and `commands/env/`): an
  `auth-command.md` parent plus `login.md`, `logout.md`, `status.md`,
  `whoami.md`; add
  them to the command toctree.
- **How-to, rewrite** `docs/source/client/how-to/authenticate-to-an-index.md`:
  lead with `sysand auth login`; demote the env-var steps to a CI / fallback
  section.
- **Explanation, update**
  `docs/source/client/explanation/authentication.md`: keyring persistence,
  read vs API surfaces, validation, the publish two-leg flow, the
  discovery-drift boundary and trust model (§8).
- **Index side, light**: cross-link `v1/whoami` from the index API
  reference if user-facing; the token pages
  (`docs/source/index/reference/api-tokens.md`,
  `how-to/create-an-api-token.md`) may gain a "use with `sysand auth login`"
  pointer.
- **CLI help**: `about`/`long_about` text for the `sysand auth` command and
  subcommands (in this repo), which the reference pages mirror.

## 14. Code placement (core vs CLI vs bindings)

The workspace splits into `core` (`sysand-core`, consumed by the CLI **and**
the py/js/java bindings; the js binding compiles to wasm) and `sysand` (the
CLI). Placement follows the repo's existing pattern: `do_*` command logic in
`core/src/commands/`, thin wrappers in `sysand/src/commands/`, optional
capabilities behind cargo features (`filesystem`, `networking`).

- **core, unconditional:** only the record types, blob JSON codec (pure
  serde), and the `CredentialStore` trait.
- **core, behind the existing `networking` feature:** the `LazyKeyringLayer`
  policy layer and its composition with the env layer, since `auth.rs` (and
  the `HTTPAuthentication` trait, defined over reqwest types) is already
  gated on `networking` (`core/src/lib.rs`). Note the wasm build is green
  today because the js binding enables neither `networking` nor
  `filesystem`, not because of keyring gating; keeping the new policy layer
  under `networking` preserves that. `SequenceAuthentication`'s fields are
  currently private, so the composition needs a public constructor (or a
  new combinator, see §9's pass-the-response-down variant).
- **core, behind a new `keyring` cargo feature** (not in `default`): the
  OS-keyring-backed `CredentialStore` impl and the cross-process file lock
  (uses `dirs`, mirroring `filesystem`). Enabled by the CLI and the py/java
  bindings; **off for the js/wasm binding**, the `keyring` crate does not
  build for wasm. A browser-side store could later implement the same trait
  (bindings/js already has local-storage machinery), but that is not v1 and
  localStorage is not secure storage.
  - **Vendored dbus.** The Linux Secret Service backend pulls libdbus;
    build it `vendored` so CI's `clippy --all-features` and contributor
    machines need no system libdbus headers. Vendoring statically embeds
    libdbus (dual AFL-2.1 OR GPL-2.0) in shipped binaries; verify with
    cargo-deny.
  - **musl exclusion.** The vendored dbus C build does not link on
    aarch64-musl (missing outline-atomics helpers), and musl targets are
    containers/CI where a Secret Service never runs. Target-gate the
    keyring dependency off musl entirely and give `OsKeyringBackend` a
    musl stub returning the backend-absent error, so the documented
    `SYSAND_CRED_*` fallback is the behavior there.
  - **Test seam.** CLI integration tests must never touch the real OS
    keyring (macOS prompts on differently-signed test binaries; CI has no
    Secret Service). Provide a deliberate seam, a debug-build-only env var
    (`SYSAND_TEST_CREDENTIAL_STORE`) selecting a file-backed or absent
    blob backend, refused loudly in release builds, and route every auth
    command and the lazy policy through it.
- **core `commands/auth.rs`, gated `all(filesystem, networking)`** (the
  same gate as publish): `do_auth_login` / `do_auth_logout` /
  `do_auth_status` orchestration (discovery fetch, glob derivation,
  validation probes, refusal rule), generic over the store trait.
  **A library call must never prompt**: the secret arrives as a parameter,
  and `validation: Option<bool>` is a plain argument, which is exactly what
  gives the bindings their clean optional keyword (§5).
- **sysand CLI:** the clap surface, hidden prompt, TTY detection,
  `--token-stdin`, default-index resolution, and user-facing messages; it
  constructs the keyring store and the composed policy and passes them into
  core, exactly like today's `auth_policy` handoff in `sysand/src/lib.rs`.

**Index reading and the keyring.** Reads in core stay generic over
`HTTPAuthentication` (`IndexEnvironmentAsync<Policy>`), so core's
index-reading code gains no keyring knowledge at compile time. At runtime
the keyring is touched only inside the policy instance the host constructed
and handed in; on wasm no such instance exists and reads behave as today.
