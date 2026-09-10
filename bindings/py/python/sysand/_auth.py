# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

"""Credentials and index configuration for calls that reach an index.

Both classes are pure configuration: they hold strings and flags, and the
Rust side builds the actual policy, resolver and HTTP client per call. That
keeps a secret out of every ``repr`` and lets one object be reused across
calls.
"""

from __future__ import annotations

import typing

_REDACTED = "<redacted>"


class AuthPolicy:
    """How to authenticate against indexes. Build one with the static
    constructors; the default for every call is :meth:`none`.

    Opening the OS credential store can raise a keychain prompt on some
    platforms, so a library call never does it unasked: only
    :meth:`from_env` with the default ``keyring=True`` opts in.
    """

    __slots__ = ("_kind", "_keyring", "_url_glob", "_secret", "_username", "_label")

    def __init__(
        self,
        kind: str,
        *,
        keyring: bool = False,
        url_glob: str | None = None,
        secret: str | None = None,
        username: str | None = None,
        label: str | None = None,
    ) -> None:
        self._kind = kind
        self._keyring = keyring
        self._url_glob = url_glob
        self._secret = secret
        self._username = username
        self._label = label

    @staticmethod
    def none() -> AuthPolicy:
        """No credentials at all; a 401/403 raises :class:`AuthError`."""
        return AuthPolicy("none")

    @staticmethod
    def from_env(*, keyring: bool = True) -> AuthPolicy:
        """The CLI's own resolution: validated ``SYSAND_CRED_<LABEL>`` groups
        (``SYSAND_CRED_<LABEL>`` holds the URL glob, ``..._BEARER_TOKEN`` or
        ``..._BASIC_USER``/``..._BASIC_PASSWORD`` the credential), composed
        with the OS credential store ``sysand auth login`` writes unless
        ``keyring=False``. Environment variables alone can never prompt.
        The variables are read when a call is made, not here."""
        return AuthPolicy("env", keyring=keyring)

    @staticmethod
    def bearer(url_glob: str, token: str, *, label: str = "python") -> AuthPolicy:
        """A bearer token for every URL matching ``url_glob``. Globs do not
        cross ``/``: use ``https://index.example/**`` for a whole host.
        ``label`` is the name error messages use for this credential."""
        return AuthPolicy("bearer", url_glob=url_glob, secret=token, label=label)

    @staticmethod
    def basic(url_glob: str, username: str, password: str) -> AuthPolicy:
        """HTTP basic credentials for every URL matching ``url_glob``."""
        return AuthPolicy(
            "basic", url_glob=url_glob, username=username, secret=password
        )

    @property
    def kind(self) -> str:
        return self._kind

    def _spec(self) -> dict[str, typing.Any]:
        return {
            "kind": self._kind,
            "keyring": self._keyring,
            "url_glob": self._url_glob,
            "secret": self._secret,
            "username": self._username,
            "label": self._label,
        }

    def __repr__(self) -> str:
        if self._kind == "none":
            return "AuthPolicy.none()"
        if self._kind == "env":
            return f"AuthPolicy.from_env(keyring={self._keyring!r})"
        if self._kind == "bearer":
            return (
                f"AuthPolicy.bearer({self._url_glob!r}, {_REDACTED}, "
                f"label={self._label!r})"
            )
        return f"AuthPolicy.basic({self._url_glob!r}, {self._username!r}, {_REDACTED})"


class Resolution:
    """Which indexes a call resolves against, mirroring the CLI's
    ``--index`` / ``--default-index`` / ``--no-index`` / ``--include-std``.

    With ``use_config`` (the default) sysand's configuration files are
    loaded and merged as the CLI does. Only the files: the CLI's environment
    overrides (``SYSAND_INDEX``, ``SYSAND_DEFAULT_INDEX``,
    ``SYSAND_CONFIG_FILE``, ``SYSAND_NO_CONFIG``) are not read.
    ``default_index`` replaces the built-in default (``https://sysand.com``)
    and any default marked in the configuration; ``index`` adds indexes that
    are tried before the defaults.
    """

    __slots__ = ("index", "default_index", "no_index", "include_std", "use_config")

    def __init__(
        self,
        *,
        index: typing.Sequence[str] = (),
        default_index: typing.Sequence[str] = (),
        no_index: bool = False,
        include_std: bool = False,
        use_config: bool = True,
    ) -> None:
        if no_index and (index or default_index):
            raise ValueError("no_index cannot be combined with index or default_index")
        self.index = list(index)
        self.default_index = list(default_index)
        self.no_index = no_index
        self.include_std = include_std
        self.use_config = use_config

    def _spec(self) -> dict[str, typing.Any]:
        return {
            "index": self.index,
            "default_index": self.default_index,
            "no_index": self.no_index,
            "include_std": self.include_std,
            "use_config": self.use_config,
        }

    def __repr__(self) -> str:
        return (
            f"Resolution(index={self.index!r}, default_index={self.default_index!r}, "
            f"no_index={self.no_index!r}, include_std={self.include_std!r}, "
            f"use_config={self.use_config!r})"
        )


__all__ = ["AuthPolicy", "Resolution"]
