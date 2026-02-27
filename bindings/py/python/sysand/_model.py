# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

from enum import Enum, auto
import typing
import datetime


class InterchangeProjectUsage(typing.TypedDict):
    resource: str
    version_constraint: typing.Optional[str]


class InterchangeProjectInfo(typing.TypedDict):
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
    created: datetime.datetime
    metamodel: typing.Optional[str]
    includes_derived: typing.Optional[bool]
    includes_implied: typing.Optional[bool]
    checksum: typing.Optional[typing.List[InterchangeProjectChecksum]]


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
    "InterchangeProjectUsage",
    "InterchangeProjectInfo",
    "InterchangeProjectChecksum",
    "InterchangeProjectMetadata",
    "CompressionMethod",
]
