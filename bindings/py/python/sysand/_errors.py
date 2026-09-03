# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations

import typing


class SysandError(RuntimeError):
    """Base class of every error raised by the ``sysand`` package.

    Subclasses ``RuntimeError`` so callers catching that keep working.

    Attributes:
        wrote: whether the failed call modified anything on disk before it
            failed. ``False`` means a clean refusal: nothing to roll back.
    """

    wrote: bool

    def __init__(self, message: str, *, wrote: bool = False) -> None:
        super().__init__(message)
        self.wrote = wrote


class ProjectError(SysandError):
    """The project (its ``.project.json``, ``.meta.json`` or location) is
    missing, malformed, or does not contain what the call needs."""


class ResolutionError(SysandError):
    """A project could not be resolved from the configured sources."""


class NotFoundError(ResolutionError):
    """No configured source knows the project."""


class SolveError(SysandError):
    """Dependency resolution found no solution.

    Attributes:
        conflicts: machine-readable participants in the failure, one dict per
            conflict with a ``"kind"`` key (``"Constraint"``, ``"NoVersions"``,
            ``"NotFound"``) and the variant's fields.
        report: the human-readable report, identical to the CLI's output.
        kind: ``"no_solution"`` or ``"retrieval"``.
    """

    conflicts: typing.List[typing.Dict[str, typing.Any]]
    report: str
    kind: str

    def __init__(
        self,
        message: str,
        *,
        wrote: bool = False,
        conflicts: typing.Sequence[typing.Dict[str, typing.Any]] = (),
        report: str = "",
        kind: str = "no_solution",
    ) -> None:
        super().__init__(message, wrote=wrote)
        self.conflicts = list(conflicts)
        self.report = report or message
        self.kind = kind


class AuthError(SysandError):
    """An index refused the request for lack of (accepted) credentials.

    The message names the URL and, when it can, the credential to fix: the
    glob that was tried, or the ``SYSAND_CRED_<LABEL>`` variable it came
    from. It never contains a secret."""


class IndexProtocolError(SysandError):
    """An index answered, but not with what the protocol requires
    (discovery document, ``versions.json``, digests, JSON shape)."""


class SyncError(SysandError):
    """``sync`` failed part-way.

    Attributes:
        partial: what was installed and pruned before the failure — a
            ``SyncOutcome`` dict — or ``None`` when nothing was.
    """

    partial: typing.Optional[typing.Dict[str, typing.Any]]

    def __init__(
        self,
        message: str,
        *,
        wrote: bool = False,
        partial: typing.Optional[typing.Dict[str, typing.Any]] = None,
    ) -> None:
        super().__init__(message, wrote=wrote)
        self.partial = partial


class EnvError(SysandError):
    """The ``.sysand`` environment is missing, unreadable or malformed."""


__all__ = [
    "SysandError",
    "ProjectError",
    "ResolutionError",
    "NotFoundError",
    "SolveError",
    "AuthError",
    "IndexProtocolError",
    "SyncError",
    "EnvError",
]
