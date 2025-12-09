import logging
import tempfile
from pathlib import Path
import re
import os
from typing import List

import pytest
from pytest_httpserver import HTTPServer

import sysand


def test_basic_new(caplog: pytest.LogCaptureFixture) -> None:
    level = logging.DEBUG
    logging.basicConfig(level=level)
    caplog.set_level(level)

    with tempfile.TemporaryDirectory() as tmpdirname:
        sysand.new("test_basic_new", "1.2.3", tmpdirname)

        assert caplog.record_tuples == [
            (
                "sysand_core.commands.init",
                logging.INFO,
                "    Creating interchange project `test_basic_new`",
            )
        ]
        with open(Path(tmpdirname) / ".project.json", "r") as f:
            assert (
                f.read()
                == '{\n  "name": "test_basic_new",\n  "version": "1.2.3",\n  "usage": []\n}\n'
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
        sysand.new("test_basic_info", "1.2.3", tmpdirname)

        info_meta = sysand.info_path(tmpdirname)
        assert info_meta is not None

        info, meta = info_meta

        assert info == {
            "name": "test_basic_info",
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
        {"name": "test_http_info", "version": "1.2.3", "usage": []}
    )
    httpserver.expect_request("/.meta.json").respond_with_json(
        {"index": {}, "created": "0000-00-00T00:00:00.123456789Z"}
    )

    info_metas = sysand.info(httpserver.url_for(""))

    assert len(info_metas) == 1
    info, meta = info_metas[0]

    assert info == {
        "name": "test_http_info",
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

    httpserver.expect_request(
        "/19148b59a7f258e6eab15189ebcc5b6f884e02690a3b27f3f43e4c6e15dd9536/versions.txt"
    ).respond_with_data("1.2.3\n")
    httpserver.expect_request(
        "/19148b59a7f258e6eab15189ebcc5b6f884e02690a3b27f3f43e4c6e15dd9536/1.2.3.kpar/.project.json"
    ).respond_with_json({"name": "test_index_info", "version": "1.2.3", "usage": []})
    httpserver.expect_request(
        "/19148b59a7f258e6eab15189ebcc5b6f884e02690a3b27f3f43e4c6e15dd9536/1.2.3.kpar/.meta.json"
    ).respond_with_json({"index": {}, "created": "0000-00-00T00:00:00.123456789Z"})

    info_metas = sysand.info(
        "urn:kpar:test_index_info", index_urls=httpserver.url_for("")
    )

    assert len(info_metas) == 1
    info, meta = info_metas[0]

    assert info == {
        "name": "test_index_info",
        "description": None,
        "version": "1.2.3",
        "license": None,
        "maintainer": [],
        "website": None,
        "topic": [],
        "usage": [],
    }

    assert meta["index"] == {}
    assert isinstance(meta["created"], str)
    assert re.match(
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.(\d{6}|\d{9})Z$", meta["created"]
    )
    assert meta["metamodel"] is None
    assert meta["includes_derived"] is None
    assert meta["includes_implied"] is None
    assert meta["checksum"] is None


def compare_sources(sources: List[str], expected_sources: List[str]) -> None:
    assert len(sources) == len(expected_sources)
    for source, expected_source in zip(sources, expected_sources):
        assert os.path.samefile(source, expected_source), (
            f"source: {source}, expected_source: {expected_source}"
        )


def test_end_to_end_install_sources():
    with tempfile.TemporaryDirectory() as tmp_main:
        with tempfile.TemporaryDirectory() as tmp_dep:
            tmp_main = Path(tmp_main).resolve()
            tmp_dep = Path(tmp_dep).resolve()
            sysand.new("test_end_to_end_install_sources", "1.2.3", tmp_main)
            sysand.new("test_end_to_end_install_sources_dep", "1.2.3", tmp_dep)

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
