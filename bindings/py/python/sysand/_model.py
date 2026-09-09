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
