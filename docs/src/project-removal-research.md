# Project removal and identifier reuse

This note summarizes how established package registries handle full project
deletion, and records a recommendation for the Sysand index and index API.

## Problem

The Sysand index protocol can represent the removal of uploaded project content,
but the policy question is whether a removed project identifier can later be
used by a different project.

The risky case is:

1. A trusted project is published under an identifier such as
   `pkg:sysand/acme/jupyterhub`.
2. The project is fully removed.
3. A different publisher later registers the same identifier and uploads
   malicious content.

Users, lockfiles, mirrors, audit tools, documentation, and dependency metadata
may continue to treat that identifier as the original project. Deletion that
frees the identifier creates a package-name takeover path.

## Registry behavior

### PyPI

PyPI currently allows deletion of projects, releases, and files by project
owners. PyPI documents that deletion is permanent and irreversible. Deleting a
project makes it uninstallable and releases the project name for use by any
other PyPI user. Deleted files cannot be re-uploaded, even after a project has
been deleted and recreated.

PyPI separately has name-retention and transfer rules under PEP 541. Abandoned
projects are not removed solely because they are abandoned; ownership transfer
requires a formal process, including failed contact attempts, an existing
notable replacement project, and maintainer approval. PyPI administrators can
also reserve or prohibit names for security reasons.

Sources:

- [PyPI help: deletion](https://pypi.org/help/#deletion)
- [PyPI help: file name reuse](https://pypi.org/help/#file-name-reuse)
- [PyPI name retention](https://docs.pypi.org/project-management/name-retention/)
- [PEP 541](https://peps.python.org/pep-0541/)

### npm

npm permits unpublishing only under constraints. Newly created packages can be
unpublished within 72 hours if no other public npm package depends on them.
Older packages can be unpublished only if all of these are true:

- no public npm packages depend on it;
- it had fewer than 300 downloads in the previous week;
- it has a single owner or maintainer.

npm does not allow a used `package@version` to be reused. If all versions are
unpublished, publishing new versions of that package name is blocked for 24
hours. npm recommends deprecation when the intent is to discourage use without
breaking consumers.

Sources:

- [npm unpublish policy](https://docs.npmjs.com/policies/unpublish/)
- [npm unpublishing docs](https://docs.npmjs.com/unpublishing-packages-from-the-registry)

### crates.io

crates.io is built around permanence. The Cargo documentation says publishing is
permanent: a version cannot be overwritten, and its code cannot be deleted by
normal user action.

`cargo yank` removes a version from new dependency resolution, but it does not
delete code. Existing lockfiles continue to work, while future lockfiles avoid
the yanked version.

Rust RFC 3660 proposes limited author-initiated crate deletion for low-impact
cases, such as crates published less than 72 hours ago, or crates with one
owner, no reverse dependencies, and very low downloads. The RFC frames this as
an exception to the registry's immutability model.

Sources:

- [Cargo publishing](https://doc.rust-lang.org/cargo/reference/publishing.html)
- [RFC 3660](https://rust-lang.github.io/rfcs/3660-crates-io-crate-deletions.html)

### NuGet

nuget.org does not support permanent deletion by normal users, because it would
break package restore workflows. It supports unlisting instead. Unlisted package
versions are hidden from search and the web UI, but remain downloadable by exact
version.

NuGet administrators can delete packages in exceptional cases such as malware,
copyright or license violations, illegal content, or identifier squatting.

Sources:

- [NuGet deleted packages policy](https://learn.microsoft.com/en-us/nuget/nuget-org/policies/deleting-packages)
- [NuGet push and delete API](https://learn.microsoft.com/en-us/nuget/api/package-publish-resource)

### Maven Central

Maven Central does not allow published release artifacts to be changed,
modified, deleted, removed, or updated. If a published artifact contains a bug,
the publisher must publish a new version.

Source:

- [Maven Central FAQ](https://central.sonatype.org/faq/can-i-change-a-component/)

### Hex

Hex treats the public repository as immutable in general. A package version can
be unpublished within 60 minutes of release, or within 24 hours of initial
package release. If the last version is reverted, the package is deleted.

Hex recommends retiring packages or releases instead of unpublishing. Retired
releases remain resolvable and fetchable, but users receive a warning and the
website marks the release as retired.

Packages or versions removed by administrators are automatically reserved and
cannot be reused by users.

Source:

- [Hex FAQ](https://hex.pm/docs/faq#can-packages-be-removed-from-the-repository)

## Recommendation for Sysand

Sysand should not allow full project deletion to free a project identifier.

Once an index accepts a canonical project identity, that identity should remain
reserved by that index. This applies to both:

- `pkg:sysand/<publisher>/<name>` identifiers; and
- non-PURL IRIs resolved through the `_iri/<sha256_hex(normalized_iri)>` bucket.

Normal maintainer actions should be modeled as state changes, not identity
deletion:

- `yank` a version to exclude it from new dependency resolution;
- `retire` or `archive` a project or version to signal that it is no longer
  recommended or maintained;
- keep exact-version or lockfile-style consumers working where possible.

Full content removal should be reserved for exceptional cases:

- malware;
- legal or intellectual-property requests;
- accidental secret exposure;
- spam or abuse;
- severe policy violations.

Even when content is removed, the index should keep a tombstone record keyed by
the canonical project identity. That tombstone should block reuse by default.

A tombstone should be able to record:

- the original project IRI;
- the normalized identity used for lookup;
- removal status;
- an optional public reason class;
- removal timestamp;
- previous advertised versions and digests, if safe to disclose;
- whether an administrator has approved restoration or ownership transfer.

Publishing should reject an upload whose canonical IRI matches a tombstoned
project unless one of these is true:

- the same authorized owner is restoring the project under index policy; or
- an administrator has explicitly transferred the project identity.

## Protocol implications

The read-side sysand index protocol should preserve a stable answer to the
question "has this index ever known this project identity?" even when the
project's downloadable bytes are no longer available.

The index API should own state-changing policy:

- who may yank;
- who may retire or archive;
- who may remove bytes;
- who may restore content;
- who may transfer a tombstoned identity.

The read-side static index can later expose tombstones explicitly, but the
important invariant should be decided first:

> Removing project content MUST NOT make the project identity available to an
> unrelated project.

This follows the safer pattern used by crates.io, NuGet, Maven Central, and Hex.
PyPI's current behavior is the example to avoid for Sysand, because full
deletion releases the name and creates an avoidable package-name takeover risk.
