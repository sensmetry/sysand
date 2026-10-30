# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

import typing
from enum import Enum, auto

# IMPORTANT
# Keep the types here in sync with Rust types from sysand-core.
#
# Use raw types for components, as these classes are converted
# to/from Rust `*Raw` model type variants.


class _InterchangeProjectUsageResourceRequired(typing.TypedDict):
    resource: str


class InterchangeProjectUsageResource(
    _InterchangeProjectUsageResourceRequired, total=False
):
    """The untyped usage KerML specifies: a project named by an IRI,
    optionally constrained to a range of versions. It exists in
    ``.project.json`` projects only: a ``model.toml`` has no such usage.
    """

    version_constraint: str | None


class InterchangeProjectUsageDirectory(typing.TypedDict):
    """A typed usage: the project in the directory ``dir``, relative to the
    root of the project declaring the usage. It carries no version
    constraint -- the directory holds a single version."""

    dir: str
    publisher: str
    name: str


class InterchangeProjectUsageKparPath(typing.TypedDict):
    """A typed usage: the project in the KPAR at ``kpar_path``, relative to
    the root of the project declaring the usage. It carries no version
    constraint -- the archive holds a single version."""

    kpar_path: str
    publisher: str
    name: str


class InterchangeProjectUsageIndex(typing.TypedDict):
    """A typed usage: the project ``publisher``/``name``, resolved from the
    index (or any other source that finds a project by its publisher and
    name), constrained to a range of versions.

    ``publisher`` and ``name`` are spelled exactly as the project spells
    them."""

    publisher: str
    name: str
    version_constraint: str


InterchangeProjectUsage = (
    InterchangeProjectUsageResource
    | InterchangeProjectUsageDirectory
    | InterchangeProjectUsageKparPath
    | InterchangeProjectUsageIndex
)


class UsageConstraintChange(typing.TypedDict):
    """Result of :func:`sysand.set_usage_constraint`."""

    found: bool
    changed: bool
    old_version_constraint: str | None
    new_version_constraint: str | None
    """The constraint now declared; ``None`` only when nothing was found."""


class EnvProjectChecksumKpar(typing.TypedDict):
    kpar_cksum: str
    """Checksum of the KPAR the project was installed from."""


class EnvProjectChecksumSrc(typing.TypedDict):
    src_cksum: str
    """Checksum of the source directory the project was installed from."""


EnvProjectChecksum = EnvProjectChecksumKpar | EnvProjectChecksumSrc


class EnvProject(typing.TypedDict):
    """One entry of an environment's ``env.toml``, see
    :func:`sysand.env.projects`."""

    publisher: str | None
    name: str
    version: str
    path: str
    """Verbatim from ``env.toml``: relative to the environment directory, or
    to the workspace/project root when ``editable``. Not joined."""
    identifiers: list[str]
    """IRIs of the project; the first is canonical. Empty only for
    ``editable`` entries."""
    usages: list[str]
    editable: bool
    workspace: bool
    checksum: EnvProjectChecksum | None
    """Checksum of what the project was installed from, or ``None`` when
    ``env.toml`` records none (``editable`` entries)."""


class Discovery(typing.TypedDict):
    """Result of :func:`sysand.discover`."""

    project_root: str | None
    """Canonical path of the enclosing project (the directory holding
    ``model.toml``, ``.project.json`` or ``.meta.json``), or ``None``."""
    workspace_root: str | None
    """Canonical path of the enclosing workspace (the directory whose
    ``model.toml`` holds a ``[workspace]`` table), or ``None``."""


class VersionListing(typing.TypedDict):
    """Result of :func:`sysand.versions`."""

    iri: str
    versions: list[str]
    """Distinct semver versions, highest first. Over an index these are the
    *available* versions: yanked and removed entries are never listed."""
    ignored: list[str]
    """Version strings that are not valid semver, in encounter order. An
    index cannot publish such entries, but other sources can."""


class SyncedProject(typing.TypedDict):
    """One project :func:`sysand.sync` installed, pruned or kept."""

    iri: str
    version: str
    path: str | None
    """Install directory relative to the environment directory, or ``None``
    once the project has been pruned."""


class Mismatch(typing.TypedDict):
    """One installed project ``sync(verify=True)`` found not to match the
    lockfile."""

    iri: str
    version: str
    kind: str
    """``"checksum"`` (files or manifests differ), ``"missing"`` (recorded
    in ``env.toml`` but not on disk) or ``"not-recorded"`` (in the lockfile
    but not in ``env.toml``)."""
    file: typing.Optional[str]
    """The first file whose digest differs, when the kind is ``checksum``
    and a file is known."""
    message: str
    """The finding as the CLI prints it."""


class StaleUsage(typing.TypedDict):
    """A manifest usage the lockfile does not reflect; the repair is
    :func:`sysand.lock`."""

    manifest: str
    usage: str
    kind: str
    """``"constraint"``, ``"added"``, ``"removed"`` or ``"manifest-missing"``."""
    message: str


class SyncOutcome(typing.TypedDict):
    """Result of :func:`sysand.sync`: every change it made, and what it left
    alone. Also carried by :class:`SyncError` as ``partial`` when a sync
    failed part-way, then without ``path``."""

    installed: list[SyncedProject]
    pruned: list[SyncedProject]
    kept: list[SyncedProject]
    """Lockfile entries that were already installed and verified."""
    verified: typing.List[SyncedProject]
    """``verify``: entries read back from disk that match."""
    mismatched: typing.List[Mismatch]
    """``verify``: entries that do not."""
    unrecorded: typing.List[str]
    """``verify``: directories under ``lib/`` that ``env.toml`` does not
    record, relative to the environment."""
    repaired: typing.List[SyncedProject]
    """``repair``: entries reinstalled."""
    stale: typing.List[StaleUsage]
    """``verify``: manifest usages the lockfile does not reflect."""


class LockedProject(typing.TypedDict):
    """One ``[[project]]`` entry of a lockfile, see :func:`sysand.lock`."""

    publisher: str | None
    name: str
    version: str
    metamodel: str | None
    """The metamodel the project declares, or ``None``. Recorded so the
    lockfile's own edge checks have something to read."""
    identifiers: list[str]
    exports: list[str]
    usages: list[str]
    """The usages this project declares, as lockfile usage strings — names
    who pins what."""
    sources: list[str]
    """Where the project can be obtained from, as the lockfile renders each
    source (opaque; for diagnostics)."""


class LockResult(typing.TypedDict):
    """Result of :func:`sysand.lock`."""

    projects: list[LockedProject]
    text: str
    """The canonical ``sysand-lock.toml`` content that was written, or would
    have been written with ``write=True``. Byte-exact, so a caller can diff
    and restore it."""


class InterchangeProjectInfo(typing.TypedDict):
    publisher: str | None
    name: str
    description: str | None
    version: str
    license: str | None
    maintainer: list[str]
    website: str | None
    topic: list[str]
    usage: list[InterchangeProjectUsage]


class InterchangeProjectChecksum(typing.TypedDict):
    value: str
    algorithm: str


class InterchangeProjectMetadata(typing.TypedDict):
    """A ``.meta.json``, in one of two shapes. An OMG-metamodel project
    carries ``index`` (symbol to file); a Syside-metamodel project carries
    ``root`` and ``exports`` instead and no ``index``. A dict read from
    Sysand has all three keys, the absent shape's as ``None``; a dict
    passed to Sysand may leave them out."""

    index: dict[str, str] | None
    root: str | None
    exports: list[str] | None
    created: str
    metamodel: str | None
    includes_derived: bool | None
    includes_implied: bool | None
    checksum: dict[str, InterchangeProjectChecksum] | None


class Dependencies(Enum):
    NONE = auto()
    """Do not list any dependency sources"""
    DEPS = auto()
    """List dependency sources, excluding standard libraries"""
    DEPS_STD = auto()
    """List dependency sources, including standard libraries"""
    STD = auto()
    """List only standard-library dependency sources"""


class CompressionMethod(Enum):
    STORED = auto()
    """Store the files as is"""
    DEFLATED = auto()
    """Compress the files using Deflate"""
    BZIP2 = auto()
    """Compress the files using BZIP2. Only available when sysand is compiled with feature kpar-bzip2"""
    ZSTD = auto()
    """Compress the files using ZStandard. Only available when sysand is compiled with feature kpar-zstd"""
    XZ = auto()
    """Compress the files using XZ. Only available when sysand is compiled with feature kpar-xz"""
    PPMD = auto()
    """Compress the files using PPMd. Only available when sysand is compiled with feature kpar-ppmd"""


__all__ = [
    "CompressionMethod",
    "Dependencies",
    "InterchangeProjectChecksum",
    "InterchangeProjectInfo",
    "InterchangeProjectMetadata",
    "InterchangeProjectUsage",
    "InterchangeProjectUsageDirectory",
    "InterchangeProjectUsageIndex",
    "InterchangeProjectUsageKparPath",
    "InterchangeProjectUsageResource",
]


class ProvidedProject(typing.TypedDict):
    """A project satisfied by the host itself, so it is never installed.
    Accepted by :func:`sysand.lock` and :func:`sysand.sync` (``provided=``);
    this is how the standard libraries are treated."""

    iri: str
    info: InterchangeProjectInfo
    meta: InterchangeProjectMetadata
