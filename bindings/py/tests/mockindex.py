# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""A local sysand index served by ``pytest_httpserver``.

The layout is the real index protocol's (see ``core/src/env/discovery.rs``):

- ``pkg:sysand/<publisher>/<name>`` lives at ``/<publisher>/<name>/``;
- any other IRI lives at ``/_iri/<sha256hex(canonical iri)>/``;
- each project directory holds ``versions.json`` and, per version,
  ``.project.json``, ``.meta.json`` and ``project.kpar``;
- ``/sysand-index-config.json`` answers 404, so the index root defaults to
  the discovery root;
- ``/index.json`` is served, but the ``mock_index`` fixture fails a test that
  fetches it (``lock`` must target ``versions.json``, never enumerate).

Nothing on the sysand side is stubbed: the CLI and the bindings talk to this
server over HTTP exactly as they talk to a live index.
"""

from __future__ import annotations

import dataclasses
import fnmatch
import hashlib
import json
import os
import re
import typing
import zipfile
from io import BytesIO
from pathlib import Path
from urllib.parse import quote

from pytest_httpserver import HTTPServer
from werkzeug.wrappers import Request, Response

from sysand._sysand_core import _run_cli  # type: ignore

# The library under test and the constraints the scenarios move between.
LIBRARY = "pkg:sysand/mock/library"
DEPENDENT = "pkg:sysand/mock/dependent"
LEGACY_CONSTRAINT = ">=0.10.0, <0.11.0"
TARGET_CONSTRAINT = ">=0.11.0, <0.12.0"

# Fixed so archive digests are reproducible across runs.
CREATED = "2026-01-01T00:00:00Z"
_ZIP_DATE_TIME = (2026, 1, 1, 0, 0, 0)

DISCOVERY_PATH = "/sysand-index-config.json"
INDEX_PATH = "/index.json"

_PURL_PREFIX = "pkg:sysand/"
_SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


def usage(resource: str, version_constraint: str | None = None) -> dict:
    """A usage entry in the manifest's wire shape (camelCase)."""
    entry: dict = {"resource": resource}
    if version_constraint is not None:
        entry["versionConstraint"] = version_constraint
    return entry


def _semver_key(version: str) -> tuple[int, int, int]:
    m = _SEMVER.match(version)
    if m is None:
        # The index protocol rejects non-semver (and pre-release/build)
        # entries at ingest (`core/src/env/index.rs::validate_versions`),
        # so the fixture refuses them rather than serving an invalid index.
        raise ValueError(
            f"mock index only publishes plain X.Y.Z versions, got {version!r}"
        )
    return tuple(int(g) for g in m.groups())  # type: ignore[return-value]


def _segments(iri: str) -> list[str]:
    """The project directory's path segments, as `project_rel_segments` builds them."""
    if iri.startswith(_PURL_PREFIX):
        publisher, name = iri[len(_PURL_PREFIX) :].split("/", 1)
        return [publisher, name]
    # Core hashes `canonicalize_iri(iri)`. The fixture hashes the IRI as
    # given, which is identical for IRIs that are already canonical
    # (`urn:...`, PURLs); pass canonical IRIs.
    return ["_iri", hashlib.sha256(iri.encode()).hexdigest()]


@dataclasses.dataclass
class _Version:
    version: str
    usage: list[dict]
    kpar: bytes
    project_json: bytes
    meta_json: bytes

    @property
    def digest(self) -> str:
        return "sha256:" + hashlib.sha256(self.kpar).hexdigest()

    def versions_entry(self) -> dict:
        return {
            "version": self.version,
            "usage": self.usage,
            "kpar_size": len(self.kpar),
            "kpar_digest": self.digest,
        }


@dataclasses.dataclass
class _Project:
    iri: str
    name: str
    publisher: str | None
    versions: dict[str, _Version] = dataclasses.field(default_factory=dict)


class MockIndex:
    def __init__(self, httpserver: HTTPServer) -> None:
        self._server = httpserver
        self._projects: dict[str, _Project] = {}
        self._files: dict[str, tuple[bytes, str]] = {}
        self._bearer: str | None = None
        self._fail_next: dict[str, int] = {}
        httpserver.expect_request(re.compile(r"^/.*$")).respond_with_handler(
            self._handle
        )

    # -- addressing ---------------------------------------------------------

    @property
    def url(self) -> str:
        return self._server.url_for("")

    @staticmethod
    def project_dir(iri: str) -> str:
        return "/" + "/".join(quote(seg, safe="") for seg in _segments(iri))

    @classmethod
    def versions_path(cls, iri: str) -> str:
        return f"{cls.project_dir(iri)}/versions.json"

    @classmethod
    def kpar_path(cls, iri: str, version: str) -> str:
        return f"{cls.project_dir(iri)}/{quote(version, safe='')}/project.kpar"

    # -- publishing ---------------------------------------------------------

    def publish(
        self,
        iri: str,
        version: str,
        *,
        usage: typing.Sequence[dict] = (),
        files: typing.Mapping[str, bytes] = {},
        index: typing.Mapping[str, str] | None = None,
        name: str | None = None,
        publisher: str | None = None,
    ) -> None:
        """Publish ``iri`` at ``version``; may be called mid-test.

        ``usage`` entries are the manifest wire shape, see :func:`usage`; they
        are written to the ``versions.json`` entry (which is what the solver
        reads) and mirrored into the archive's ``.project.json``.

        ``files`` go into the archive and, unless ``index`` is given, are
        listed in ``.meta.json``'s symbol index under their stem — ``sync``
        installs only indexed source files.
        """
        if index is None:
            index = {Path(path).stem: path for path in files}
        _semver_key(version)
        if iri.startswith(_PURL_PREFIX):
            default_publisher, default_name = _segments(iri)
        else:
            default_publisher, default_name = None, re.split(r"[:/]", iri)[-1]
        project = self._projects.setdefault(
            iri,
            _Project(
                iri=iri,
                name=name or default_name,
                publisher=publisher or default_publisher,
            ),
        )
        if version in project.versions:
            raise ValueError(f"{iri} {version} is already published")

        info: dict = {"name": project.name, "version": version}
        if project.publisher is not None:
            info["publisher"] = project.publisher
        if usage:
            info["usage"] = list(usage)
        meta = {"index": dict(index), "created": CREATED}
        project_json = (json.dumps(info, indent=2) + "\n").encode()
        meta_json = (json.dumps(meta, indent=2) + "\n").encode()

        buf = BytesIO()
        with zipfile.ZipFile(buf, "w", compression=zipfile.ZIP_STORED) as zf:
            for arcname, data in [
                (".project.json", project_json),
                (".meta.json", meta_json),
                *files.items(),
            ]:
                zi = zipfile.ZipInfo(arcname, date_time=_ZIP_DATE_TIME)
                zi.compress_type = zipfile.ZIP_STORED
                zi.external_attr = 0o644 << 16
                zf.writestr(zi, data)

        entry = _Version(
            version=version,
            usage=list(usage),
            kpar=buf.getvalue(),
            project_json=project_json,
            meta_json=meta_json,
        )
        project.versions[version] = entry

        base = f"{self.project_dir(iri)}/{quote(version, safe='')}"
        self._files[f"{base}/.project.json"] = (project_json, "application/json")
        self._files[f"{base}/.meta.json"] = (meta_json, "application/json")
        self._files[f"{base}/project.kpar"] = (entry.kpar, "application/zip")
        self._refresh_versions_json(project)

    def _refresh_versions_json(self, project: _Project) -> None:
        # Ingest validation requires strictly descending semver order.
        ordered = sorted(
            project.versions.values(),
            key=lambda v: _semver_key(v.version),
            reverse=True,
        )
        body = json.dumps({"versions": [v.versions_entry() for v in ordered]})
        self._files[self.versions_path(project.iri)] = (
            body.encode(),
            "application/json",
        )

    def kpar_digest(self, iri: str, version: str) -> str:
        return self._projects[iri].versions[version].digest

    def kpar_size(self, iri: str, version: str) -> int:
        return len(self._projects[iri].versions[version].kpar)

    # -- fault injection ----------------------------------------------------

    def require_bearer(self, token: str) -> None:
        """Answer 401 to every request without ``Authorization: Bearer <token>``."""
        self._bearer = token

    def fail_next(self, path: str, status: int) -> None:
        """Make the next request for ``path`` fail with ``status``."""
        self._fail_next[path] = status

    # -- observation --------------------------------------------------------

    def requests(self, path_glob: str = "*") -> list[str]:
        """Paths requested so far, in order, filtered by an fnmatch glob."""
        return [
            req.path
            for req, _resp in self._server.log
            if fnmatch.fnmatchcase(req.path, path_glob)
        ]

    # -- dispatch -----------------------------------------------------------

    def _handle(self, request: Request) -> Response:
        path = request.path
        if self._bearer is not None:
            if request.headers.get("Authorization") != f"Bearer {self._bearer}":
                return Response(
                    "unauthorized",
                    status=401,
                    headers={"WWW-Authenticate": "Bearer"},
                )
        status = self._fail_next.pop(path, None)
        if status is not None:
            return Response(f"injected failure {status}", status=status)
        if path == DISCOVERY_PATH:
            return Response("", status=404)
        if path == INDEX_PATH:
            body = json.dumps({"projects": [{"iri": iri} for iri in self._projects]})
            return Response(body, status=200, content_type="application/json")
        entry = self._files.get(path)
        if entry is None:
            return Response("not found", status=404)
        body, content_type = entry
        return Response(body, status=200, content_type=content_type)


def run_cli_in(root: str | Path, *args: str) -> bool:
    """Run the in-process CLI with ``root`` as its working directory.

    ``run_cli`` discovers the project from the process cwd, so the cwd is
    switched for the duration of the call (process-global, visible to Rust).
    """
    previous = os.getcwd()
    os.chdir(root)
    try:
        return bool(_run_cli(["sysand", *args]))
    finally:
        os.chdir(previous)
