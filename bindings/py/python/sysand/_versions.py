# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import sysand._sysand_core as sysand_rs  # type: ignore

from ._auth import AuthPolicy, Resolution
from ._model import VersionListing


def versions(
    iri: str,
    *,
    resolution: Resolution | None = None,
    auth: AuthPolicy | None = None,
) -> VersionListing:
    """List the published versions of the project ``iri``.

    The sibling of :func:`info`: the same resolution, but every candidate's
    version is collected instead of keeping the best one. Over an index this
    costs one ``versions.json`` request per index.

    ``resolution`` defaults to :class:`Resolution` ``()``, i.e. the CLI's
    behaviour: configuration files plus the default index. Pass an explicit
    :class:`Resolution` to control which indexes are consulted. ``auth``
    defaults to :meth:`AuthPolicy.none`.

    Raises:
        NotFoundError: no configured source knows ``iri``.
        AuthError: an index refused the request (HTTP 401/403).
        IndexProtocolError: an index answered with something malformed.
        ResolutionError: any other resolution failure.
    """
    if resolution is None:
        resolution = Resolution()
    iri_out, found, ignored = sysand_rs.do_versions_py(
        iri, resolution._spec(), auth._spec() if auth is not None else None
    )
    return VersionListing(iri=iri_out, versions=list(found), ignored=list(ignored))


__all__ = ["versions"]
