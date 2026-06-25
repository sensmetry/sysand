# Sysand Index API Protocol

> **Status: partial.** The upload endpoint is described at a high level;
> the rest of the API surface is not yet specified.

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
  in the index protocol). When `api_root` is absent, clients default it
  to the discovery root.

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
{ "token": "<sysand-index-bearer-token>" }
```

Clients use the returned bearer token for subsequent publish API calls,
including `POST v1/upload`. The provider OIDC token and returned bearer
token are secrets and clients MUST NOT log them.

Non-2xx responses indicate that the exchange failed. Servers SHOULD use
401 or 403 when the provider token is invalid, expired, has the wrong
audience, or is not authorized to publish the target project.

Initially supported CI providers:

- GitHub Actions on github.com. The client requests an OIDC token from
  the GitHub runner OIDC endpoint with audience `sysand`.
- GitLab CI on gitlab.com. The job must expose an ID token with audience
  `sysand`, for example through `id_tokens`.

The client discovers `api_root` before attempting trusted publishing.
Therefore trusted publishing does not help with an auth-gated
`sysand-index-config.json`; such discovery still requires separately
configured credentials.
