# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

import logging
import tempfile
from pathlib import Path
import re
import os
from typing import List, Union

import pytest
from pytest_httpserver import HTTPServer

import sysand


def test_basic_init(caplog: pytest.LogCaptureFixture) -> None:
    level = logging.DEBUG
    logging.basicConfig(level=level)
    caplog.set_level(level)

    with tempfile.TemporaryDirectory() as tmpdirname:
        sysand.init("test_basic_init", "a", "1.2.3", tmpdirname)

        assert caplog.record_tuples == [
            (
                "sysand_core.commands.init",
                logging.INFO,
                "    Creating interchange project `test_basic_init`",
            )
        ]
        with open(Path(tmpdirname) / ".project.json", "r") as f:
            assert (
                f.read()
                == '{\n  "name": "test_basic_init",\n  "publisher": "a",\n  "version": "1.2.3",\n  "usage": []\n}\n'
            )
        with open(Path(tmpdirname) / ".meta.json", "r") as f:
            assert re.match(
                r'\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{6}|\d{9})Z"\n}\n',
                f.read(),
            )


def test_basic_env() -> None:
    with tempfile.TemporaryDirectory() as tmpdirname:
        env_path = Path(tmpdirname) / sysand.env.DEFAULT_ENV_NAME
        sysand.env.env(env_path)
        assert env_path.is_dir()
        assert (env_path / "entries.txt").is_file()
        assert os.stat(env_path / "entries.txt").st_size == 0


def test_basic_info(caplog: pytest.LogCaptureFixture) -> None:
    level = logging.DEBUG
    logging.basicConfig(level=level)
    caplog.set_level(level)

    with tempfile.TemporaryDirectory() as tmpdirname:
        sysand.init("test_basic_info", "a", "1.2.3", tmpdirname)

        info_meta = sysand.info_path(tmpdirname)
        assert info_meta is not None

        info, meta = info_meta

        assert info == {
            "name": "test_basic_info",
            "publisher": "a",
            "description": None,
            "version": "1.2.3",
            "license": None,
            "maintainer": [],
            "website": None,
            "topic": [],
            "usage": [],
        }

        assert meta["index"] == {}
        # Python's datetime.fromisoformat() does not support nanoseconds yet, so
        # we check the validity of the string using a regex.
        assert isinstance(meta["created"], str)
        assert re.match(
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.(\d{6}|\d{9})Z$", meta["created"]
        )
        assert meta["metamodel"] is None
        assert meta["includes_derived"] is None
        assert meta["includes_implied"] is None
        assert meta["checksum"] is None

        file_uri = Path(tmpdirname).resolve().as_uri()

        info_metas = sysand.info(file_uri)
        assert len(info_metas) == 1, f"file_uri: {file_uri}"
        assert info_metas[0] == info_meta


def test_http_info(caplog: pytest.LogCaptureFixture, httpserver: HTTPServer) -> None:
    level = logging.DEBUG
    logging.basicConfig(level=level)
    caplog.set_level(level)

    httpserver.expect_request("/.project.json").respond_with_json(
        {"name": "test_http_info", "publisher": "a", "version": "1.2.3", "usage": []}
    )
    httpserver.expect_request("/.meta.json").respond_with_json(
        {"index": {}, "created": "0000-00-00T00:00:00.123456789Z"}
    )

    info_metas = sysand.info(httpserver.url_for(""))

    assert len(info_metas) == 1
    info, meta = info_metas[0]

    assert info == {
        "name": "test_http_info",
        "publisher": "a",
        "description": None,
        "version": "1.2.3",
        "license": None,
        "maintainer": [],
        "website": None,
        "topic": [],
        "usage": [],
    }

    assert meta["index"] == {}
    # meta['created'] utc time
    assert meta["metamodel"] is None
    assert meta["includes_derived"] is None
    assert meta["includes_implied"] is None
    assert meta["checksum"] is None


def test_index_info(caplog: pytest.LogCaptureFixture, httpserver: HTTPServer) -> None:
    level = logging.DEBUG
    logging.basicConfig(level=level)
    caplog.set_level(level)

    # `urn:kpar:test_index_info` does not match the `pkg:sysand/<pub>/<name>`
    # layout, so the client looks it up under `_iri/<sha256(iri)>/…`.
    # `project_digest` must match the canonical digest computed from the
    # served `.project.json`/`.meta.json` pair; the client verifies it before
    # surfacing results. `kpar_digest` is not verified here because
    # `sysand.info` doesn't download the archive.
    project_digest = (
        "sha256:51f51e675232511a4ccded32c9bb92d2443f68cf2265a460f74204655547b409"
    )
    filler_digest = "sha256:" + ("a" * 64)
    # On first contact, the client fetches the well-known discovery
    # document (`/.well-known/sysand-index.json`). A 404 tells it to fall
    # back to the discovery root as the index root.
    httpserver.expect_request("/.well-known/sysand-index.json").respond_with_data(
        "", status=404
    )
    iri_dir = "/_iri/19148b59a7f258e6eab15189ebcc5b6f884e02690a3b27f3f43e4c6e15dd9536"
    httpserver.expect_request(f"{iri_dir}/versions.json").respond_with_json(
        {
            "versions": [
                {
                    "version": "1.2.3",
                    "usage": [],
                    "project_digest": project_digest,
                    "kpar_size": 42,
                    "kpar_digest": filler_digest,
                }
            ]
        }
    )
    httpserver.expect_request(f"{iri_dir}/1.2.3/.project.json").respond_with_json(
        {"name": "test_index_info", "version": "1.2.3", "usage": []}
    )
    httpserver.expect_request(f"{iri_dir}/1.2.3/.meta.json").respond_with_json(
        {"index": {}, "created": "2026-01-01T00:00:00.000000000Z"}
    )

    info_metas = sysand.info(
        "urn:kpar:test_index_info", index_urls=httpserver.url_for("")
    )

    assert len(info_metas) == 1
    info, meta = info_metas[0]

    assert info["name"] == "test_index_info"
    assert info["version"] == "1.2.3"
    assert info["usage"] == []
    assert info["publisher"] is None

    assert meta["index"] == {}
    assert isinstance(meta["created"], str)
    assert re.match(
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.(\d{6}|\d{9})Z$", meta["created"]
    )
    assert meta["metamodel"] is None
    assert meta["includes_derived"] is None
    assert meta["includes_implied"] is None
    assert meta["checksum"] is None


def compare_sources(
    sources: Union[List[Path], List[str]],
    expected_sources: Union[List[Path], List[str]],
) -> None:
    assert len(sources) == len(expected_sources)
    for source, expected_source in zip(sources, expected_sources):
        assert os.path.samefile(source, expected_source), (
            f"source: {source}, expected_source: {expected_source}"
        )


def test_end_to_end_install_sources() -> None:
    with tempfile.TemporaryDirectory() as tmp_main:
        with tempfile.TemporaryDirectory() as tmp_dep:
            tmp_main = Path(tmp_main).resolve()
            tmp_dep = Path(tmp_dep).resolve()
            sysand.init("test_end_to_end_install_sources", "a", "1.2.3", tmp_main)
            sysand.init("test_end_to_end_install_sources_dep", "a", "1.2.3", tmp_dep)

            with open(Path(tmp_main) / "src.sysml", "w") as f:
                f.write("package Src;")

            sysand.include(tmp_main, "src.sysml")

            with open(Path(tmp_dep) / "src_dep.sysml", "w") as f:
                f.write("package SrcDep;")

            sysand.include(tmp_dep, "src_dep.sysml")

            env_path = Path(tmp_main) / sysand.env.DEFAULT_ENV_NAME

            sysand.env.env(env_path)

            sysand.env.install_path(
                env_path, "urn:kpar:test_end_to_end_install_sources_dep", tmp_dep
            )

            sysand.add(
                tmp_main, "urn:kpar:test_end_to_end_install_sources_dep", "1.2.3"
            )

            compare_sources(
                sysand.sources(tmp_main, include_deps=False),
                [str(Path(tmp_main) / "src.sysml")],
            )
            compare_sources(
                sysand.sources(tmp_dep, include_deps=False),
                [str(Path(tmp_dep) / "src_dep.sysml")],
            )
            compare_sources(
                sysand.sources(tmp_main, include_deps=True, env_path=env_path),
                [
                    str(Path(tmp_main) / "src.sysml"),
                    str(
                        env_path
                        / "2e64090eee0e6ed625d7f8bbf4611d28c8d307e69cf433877b629743eb352660"
                        / "1.2.3.kpar"
                        / "src_dep.sysml"
                    ),
                ],
            )

            sysand.exclude(tmp_main, "src.sysml")

            compare_sources(
                sysand.sources(tmp_main, include_deps=True, env_path=env_path),
                [
                    str(
                        env_path
                        / "2e64090eee0e6ed625d7f8bbf4611d28c8d307e69cf433877b629743eb352660"
                        / "1.2.3.kpar"
                        / "src_dep.sysml"
                    ),
                ],
            )


@pytest.mark.parametrize(
    "compression",
    [None, sysand.CompressionMethod.STORED, sysand.CompressionMethod.DEFLATED],
)
def test_build(compression: Union[sysand.CompressionMethod, None]) -> None:
    with tempfile.TemporaryDirectory() as tmp_main:
        tmp_main = Path(tmp_main).resolve()
        sysand.init("test_build", "a", "1.2.3", tmp_main)

        with open(tmp_main / "src.sysml", "w") as f:
            f.write("package Src;")

        sysand.include(tmp_main, "src.sysml")

        sysand.build(
            output_path=tmp_main / "test_build.kpar",
            project_path=tmp_main,
            compression=compression,
        )
