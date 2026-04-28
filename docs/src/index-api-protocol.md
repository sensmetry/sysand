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
