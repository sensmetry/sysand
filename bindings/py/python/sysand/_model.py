# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

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


__all__ = [
    "InterchangeProjectUsage",
    "InterchangeProjectInfo",
    "InterchangeProjectChecksum",
    "InterchangeProjectMetadata",
]
