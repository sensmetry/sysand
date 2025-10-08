# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

import argparse
import json
from pathlib import Path
import platform
import shutil
import subprocess
from typing import Any


ROOT_DIR = Path(__file__).absolute().parent.parent.parent.parent
TARGET_DIR = ROOT_DIR / "target"
BUILD_DIR = TARGET_DIR / "java"
TEST_DIR = TARGET_DIR / "java-test"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    build_parser = subparsers.add_parser("build", help="Build the Java library.")
    build_parser.add_argument("--release", action="store_true")
    _test_parser = subparsers.add_parser("test", help="Test the Java library.")
    return parser.parse_args()


def mvn_executable() -> str:
    if platform.system() == "Windows":
        return "mvn.cmd"
    return "mvn"


def execute(command: "list[str]", *args: Any, **kwargs: Any) -> None:
    print("Executing:", " ".join(command))
    try:
        output = subprocess.check_output(
            command, *args, **kwargs, text=True, stderr=subprocess.STDOUT
        )
    except subprocess.CalledProcessError as error:
        print("Error:", error.returncode)
        print(error.output)
        raise error
    print("Success")
    print(output)


def parse_version() -> str:
    output = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            ROOT_DIR / "Cargo.toml",
        ],
        text=True,
    )
    metadata = json.loads(output)
    for package in metadata["packages"]:
        if package["name"] == "sysand-java":
            return package["version"]
    raise ValueError("sysand-java not found in Cargo.toml")


def build(release: bool, version: str) -> None:
    print("Cleaning the target directory...")
    shutil.rmtree(BUILD_DIR, ignore_errors=True)
    BUILD_DIR.mkdir(parents=True, exist_ok=True)

    print("Building the native Java library...")
    args = ["cargo", "build", "--package", "sysand-java"]
    if release:
        args.append("--release")
    execute(args, cwd=BUILD_DIR)

    print("Copying the Java code to the target directory...")
    shutil.copytree(ROOT_DIR / "bindings" /"java" / "java" / "src", BUILD_DIR / "src")

    print("Copying the native library to the target directory...")
    if platform.system() == "Darwin":
        lib_name = "libsysand.dylib"
    elif platform.system() == "Linux":
        lib_name = "libsysand.so"
    elif platform.system() == "Windows":
        lib_name = "sysand.dll"
    else:
        raise ValueError(f"Unsupported platform: {platform.system()}")
    native_lib_build_dir_name = "release" if release else "debug"
    native_lib_build_path = TARGET_DIR / native_lib_build_dir_name / lib_name
    native_lib_target_dir = BUILD_DIR / "src" / "main" / "resources"
    native_lib_target_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(native_lib_build_path, native_lib_target_dir / lib_name)

    print(
        "Copying the `pom.xml` template to the target directory and replacing the version..."
    )
    pom_path = ROOT_DIR / "bindings" / "java" / "java" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = BUILD_DIR / "pom.xml"
    target_pom_data = pom_data.replace("VERSION", version)
    target_pom_path.write_text(target_pom_data)

    print("Building the JAR...")
    execute(
        [mvn_executable(), "clean", "install", "compile", "assembly:single", "-U"],
        cwd=BUILD_DIR,
    )


def test(version: str) -> None:
    print("Looking for Java library...")
    jar_path = BUILD_DIR / "target" / f"sysand-{version}.jar"
    if not jar_path.exists():
        raise FileNotFoundError(f"JAR file not found: {jar_path}")

    print("Copying the test code to the target directory...")
    shutil.rmtree(TEST_DIR, ignore_errors=True)
    shutil.copytree(ROOT_DIR / "bindings" / "java" / "java-test", TEST_DIR)

    print("Replacing org.sysand.sysand dependency version in test pom.xml")
    pom_path = ROOT_DIR / "bindings" / "java" / "java-test" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = TEST_DIR / "pom.xml"
    target_pom_data = pom_data.replace("SYSAND_VERSION", version)
    target_pom_path.write_text(target_pom_data)

    print("Testing the Java library...")
    execute([mvn_executable(), "test"], cwd=TEST_DIR)


def main() -> None:
    args = parse_args()
    print("ROOT_DIR:", ROOT_DIR)
    print("BUILD_DIR:", BUILD_DIR)
    print("TEST_DIR:", TEST_DIR)
    version = parse_version()
    print("Version:", version)
    if args.command == "build":
        build(args.release, version)
    elif args.command == "test":
        test(version)


if __name__ == "__main__":
    main()
