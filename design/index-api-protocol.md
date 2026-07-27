# Sysand Index API Protocol

> **Status: partial.** The upload endpoint is described at a high level;
> the trusted publishing token exchange and `v1/whoami` are specified
> below; the rest of the API surface is not yet specified.

## Scope

This document will specify the **sysand index API** — the dynamic HTTP
endpoints a sysand index server MAY expose for operations that change
server state. Expected subjects include:

- Publishing a new project version (`POST v1/upload` and related).
- Authentication and authorization (bearer tokens; see
  [Authentication](authentication.md)).
- Any future management operations for `yanked` and `removed` versions.

### Relationship to the sysand index protocol

The [sysand index protocol](index-protocol.md) specifies the read-side
contract: the static file tree (`index.json`, per-project `versions.json`,
per-version `.project.json` / `.meta.json` / `project.kpar`), its
discovery document, and the verification rules a client applies.

The sysand index API is complementary:

- An index server MAY expose only the read-side sysand index (a plain
  static-file mirror is a conforming sysand index server with no API).
- An index server MAY expose both the sysand index and the sysand index
  API.
- Clients discover the API's base URL via `api_root` in
  `sysand-index-config.json` (see
  [§3 Discovery and configuration](index-protocol.md#3-discovery-and-configuration)
  in the index protocol). `api_root` has no default: when it is absent
  the server exposes no API, so the index is read-only. A server exposes
  an API only by advertising `api_root` explicitly.

The two protocols share terminology and configuration, but a server MAY
conform to one without conforming to the other.

## Status

The sysand index API is in use today — `sysand publish` posts to
`v1/upload` under the resolved `api_root` — but has not yet been written
up as a standalone spec.
This document will be populated as the API surface is formalized.

## Trusted Publishing Token Exchange

An index server that supports CI trusted publishing exposes:

```text
POST v1/oidc/token
```

under the resolved `api_root`. The endpoint accepts a short-lived OIDC
token issued by a supported CI provider and returns a short-lived
Sysand index bearer token scoped for publishing.

Request body:

```json
{ "token": "<provider-oidc-token>" }
```

Successful response body:

```json
{
  "token": "<sysand-index-bearer-token>",
  "expires_at": "<iso-8601-expiration>"
}
```

Clients use the returned bearer token for subsequent publish API calls,
including `POST v1/upload`. The returned token is short lived, scoped to
the matched project, and consumed by the upload request. The provider
OIDC token and returned bearer token are secrets and clients MUST NOT log
them.

Error response body:

```json
{ "error": "<message>" }
```

Non-2xx responses indicate that the exchange failed. Servers SHOULD use
400 when the request body is malformed or lacks a string `token` field,
403 when the provider token is invalid, expired, has the wrong audience,
comes from an unsupported issuer, lacks required CI claims, or does not
match any configured trusted publisher, and 429 when the unauthenticated
exchange endpoint is rate limited. Error messages are intended for
humans; clients SHOULD surface the HTTP status and message but MUST NOT
depend on exact message text for control flow.

Initially supported CI providers:

- GitHub Actions on github.com. The client requests an OIDC token from
  the GitHub runner OIDC endpoint with audience `sysand`.
- GitLab CI on gitlab.com. The job must expose an ID token with audience
  `sysand`, for example through `id_tokens`.

The client discovers `api_root` before attempting trusted publishing.
Therefore trusted publishing does not help with an auth-gated
`sysand-index-config.json`; such discovery still requires separately
configured credentials.

## Token Identity

An index server that supports bearer-token authentication exposes:

```text
GET v1/whoami
```

under the resolved `api_root`. The endpoint returns the identity behind
a bearer token, so a client can validate a stored credential and report
who it authenticates as (for example in `sysand auth status`).

The request carries the credential in an `Authorization: Bearer <token>`
header and has no body. Only `GET` is accepted; servers SHOULD reject
other methods with 405.

Successful response body:

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

`subject` identifies the principal the token authenticates as:

- `subject.type` is `user`, `project`, or `oidc`.
- `subject.name` depends on the token type: for a user token it is the
  owning user's username; for a project token it is the project id of
  the scoped project as normalized `<publisher-id>/<name-id>`; for a
  trusted-publishing (`oidc`) token it is the matched trusted
  publisher's path (for example the repository full name on GitHub or
  the project path on GitLab), with a `<provider>:<immutable-id>`
  fallback when the publisher configuration no longer exists.

`token` describes the credential itself:

- `token.name` is the user-given token label, or `null` for tokens that
  have no label (exchanged trusted-publishing tokens).
- `token.prefix` is the token's non-secret display prefix.
- `token.expires_at` is the token's expiration time as an ISO 8601 UTC
  timestamp with a `Z` suffix.

A missing, malformed, unrecognized, expired, or already-consumed token
yields 401. The 401 response body is unspecified; clients MUST NOT
depend on its contents and SHOULD treat any 401 as the credential being
rejected.

The endpoint is read-only and MUST NOT consume single-use credentials:
presenting an exchanged trusted-publishing token to `v1/whoami` leaves
it usable for the subsequent upload. Servers SHOULD rate limit the
endpoint (429), since it can be probed without a valid credential and
acts as a token-validity oracle.
