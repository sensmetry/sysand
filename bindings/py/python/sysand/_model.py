# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2025-2026 Sysand contributors <opensource@sensmetry.com>

from enum import Enum, auto
import typing

# IMPORTANT
# Keep the types here in sync with Rust types from sysand-core.
#
# Use raw types for components, as these classes are converted
# to/from Rust `*Raw` model type variants


class InterchangeProjectUsageResource(typing.TypedDict):
    resource: str
    version_constraint: typing.Optional[str]


class InterchangeProjectUsageDirectory(typing.TypedDict):
    dir: str
    publisher: str
    name: str


class InterchangeProjectUsageKparPath(typing.TypedDict):
    kpar_path: str
    publisher: str
    name: str


InterchangeProjectUsage = typing.Union[
    InterchangeProjectUsageResource,
    InterchangeProjectUsageDirectory,
    InterchangeProjectUsageKparPath,
]


class UsageConstraintChange(typing.TypedDict):
    """Result of :func:`sysand.set_usage_constraint`."""

    resource: str
    """The resource actually matched, with the ``publisher/name`` shorthand
    expanded to its ``pkg:sysand/`` IRI."""
    found: bool
    changed: bool
    old_constraint: typing.Optional[str]
    new_constraint: typing.Optional[str]


class EnvProjectChecksumKpar(typing.TypedDict):
    kpar_cksum: str
    """Checksum of the KPAR the project was installed from."""


class EnvProjectChecksumSrc(typing.TypedDict):
    src_cksum: str
    """Checksum of the source directory the project was installed from."""


EnvProjectChecksum = typing.Union[EnvProjectChecksumKpar, EnvProjectChecksumSrc]


class EnvProject(typing.TypedDict):
    """One entry of an environment's ``env.toml``, see
    :func:`sysand.env.projects`."""

    publisher: typing.Optional[str]
    name: str
    version: str
    path: str
    """Verbatim from ``env.toml``: relative to the environment directory, or
    to the workspace/project root when ``editable``. Not joined."""
    identifiers: typing.List[str]
    """IRIs of the project; the first is canonical. Empty only for
    ``editable`` entries."""
    usages: typing.List[str]
    editable: bool
    workspace: bool
    checksum: typing.Optional[EnvProjectChecksum]
    """Checksum of what the project was installed from, or ``None`` when
    ``env.toml`` records none (``editable`` entries)."""


class Discovery(typing.TypedDict):
    """Result of :func:`sysand.discover`."""

    project_root: typing.Optional[str]
    """Canonical path of the enclosing project (the directory holding
    ``.project.json`` or ``.meta.json``), or ``None``."""
    workspace_root: typing.Optional[str]
    """Canonical path of the enclosing workspace (the directory holding
    ``.workspace.json``), or ``None``."""


class VersionListing(typing.TypedDict):
    """Result of :func:`sysand.versions`."""

    iri: str
    versions: typing.List[str]
    """Distinct semver versions, highest first. Over an index these are the
    *available* versions: yanked and removed entries are never listed."""
    ignored: typing.List[str]
    """Version strings that are not valid semver, in encounter order. An
    index cannot publish such entries, but other sources can."""


class SyncedProject(typing.TypedDict):
    """One project :func:`sysand.sync` installed, pruned or kept."""

    iri: str
    version: str
    path: typing.Optional[str]
    """Install directory relative to the environment directory, or ``None``
    once the project has been pruned."""


class SyncOutcome(typing.TypedDict):
    """Result of :func:`sysand.sync`: every change it made, and what it left
    alone. Also carried by :class:`SyncError` as ``partial`` when a sync
    failed part-way."""

    installed: typing.List[SyncedProject]
    pruned: typing.List[SyncedProject]
    kept: typing.List[SyncedProject]
    """Lockfile entries that were already installed and verified."""


class LockedProject(typing.TypedDict):
    """One ``[[project]]`` entry of a lockfile, see :func:`sysand.lock`."""

    publisher: typing.Optional[str]
    name: str
    version: str
    identifiers: typing.List[str]
    exports: typing.List[str]
    usages: typing.List[str]
    """The usages this project declares, as lockfile usage strings — names
    who pins what."""
    sources: typing.List[str]
    """Where the project can be obtained from, as the lockfile renders each
    source (opaque; for diagnostics)."""


class LockResult(typing.TypedDict):
    """Result of :func:`sysand.lock`."""

    projects: typing.List[LockedProject]
    text: str
    """The canonical ``sysand-lock.toml`` content that was written, or would
    have been written with ``write=True``. Byte-exact, so a caller can diff
    and restore it."""


class InterchangeProjectInfo(typing.TypedDict):
    publisher: typing.Optional[str]
    name: str
    description: typing.Optional[str]
    version: str
    license: typing.Optional[str]
    maintainer: typing.List[str]
    website: typing.Optional[str]
    topic: typing.List[str]
    usage: typing.List[InterchangeProjectUsage]


class InterchangeProjectChecksum(typing.TypedDict):
    value: str
    algorithm: str


class InterchangeProjectMetadata(typing.TypedDict):
    index: typing.Dict[str, str]
    created: str
    metamodel: typing.Optional[str]
    includes_derived: typing.Optional[bool]
    includes_implied: typing.Optional[bool]
    checksum: typing.Optional[typing.Dict[str, InterchangeProjectChecksum]]


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
    "InterchangeProjectUsageResource",
    "InterchangeProjectUsageDirectory",
    "InterchangeProjectUsageKparPath",
    "InterchangeProjectUsage",
    "InterchangeProjectInfo",
    "InterchangeProjectChecksum",
    "InterchangeProjectMetadata",
    "Dependencies",
    "CompressionMethod",
]


class ProvidedProject(typing.TypedDict):
    """A project satisfied by the host itself, so it is never installed.
    Accepted by :func:`sysand.lock` and :func:`sysand.sync` (``provided=``);
    this is how the standard libraries are treated."""

    iri: str
    info: InterchangeProjectInfo
    meta: InterchangeProjectMetadata
