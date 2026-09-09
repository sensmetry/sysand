# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

from __future__ import annotations


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


class EnvError(SysandError):
    """The ``.sysand`` environment is missing, unreadable or malformed."""


__all__ = [
    "SysandError",
    "ProjectError",
    "EnvError",
]
