#!/usr/bin/env python3
#
# SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
from typing import Any


ROOT_DIR = Path(__file__).absolute().parent.parent.parent.parent
TARGET_DIR = ROOT_DIR / "target"
BUILD_DIR = TARGET_DIR / "java"
TEST_DIR = TARGET_DIR / "java-test"
DEPLOY_TEST_DIR = TARGET_DIR / "java-deploy-test"
PLUGIN_DIR = TARGET_DIR / "java-plugin"
VERSION_FILE = ROOT_DIR / "version.txt"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--release-jar-version",
        action="store_true",
        help="Produce a non-snapshot version of the JAR.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    build_parser = subparsers.add_parser("build", help="Build the Java library.")
    build_parser.add_argument(
        "--use-release-build",
        action="store_true",
        help="Whether to use the release build of the native library.",
    )
    build_parser.add_argument(
        "--use-existing-native-libs",
        type=Path,
        default=None,
        help="Path to the directory containing the native libraries. If not provided, the native libraries will be built using cargo.",
    )
    build_plugin_parser = subparsers.add_parser(
        "build-plugin", help="Build the sysand Maven plugin."
    )
    build_plugin_parser.add_argument(
        "--release-jar-version",
        action="store_true",
        help="Produce a non-snapshot version of the JAR.",
    )
    _test_parser = subparsers.add_parser("test", help="Test the Java library.")
    _deploy_parser = subparsers.add_parser(
        "deploy", help="Deploy the Java library to a Maven repository."
    )
    _test_deployed_parser = subparsers.add_parser(
        "test-deployed", help="Test the deployed Java library."
    )
    _create_version_file_parser = subparsers.add_parser(
        "create-version-file", help="Create the version file."
    )
    return parser.parse_args()


def mvn_executable() -> str:
    if platform.system() == "Windows":
        return "mvn.cmd"
    return "mvn"


def execute(command: "list[str]", *args: Any, **kwargs: Any) -> None:
    print("Executing:", " ".join(command))
    try:
        output = subprocess.check_output(  # type: ignore[call-overload]
            command, *args, **kwargs, text=True, stderr=subprocess.STDOUT
        )
    except subprocess.CalledProcessError as error:
        print("Error:", error.returncode)
        print(error.output)
        raise error
    print("Success")
    print(output)


def parse_version() -> str:
    if VERSION_FILE.exists():
        print("Using version from version.txt")
        return VERSION_FILE.read_text().strip()
    print("Computing version from Cargo.toml")
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
            version = package["version"]
            return version
    raise ValueError("sysand-java not found in Cargo.toml")


def create_version_file(version: str) -> None:
    VERSION_FILE.write_text(version)


def compute_full_version(version: str, release_jar_version: bool) -> str:
    if release_jar_version:
        return version
    return f"{version}-SNAPSHOT"


def build(
    use_release_build: bool,
    use_existing_native_libs: Path | None,
    release_jar_version: bool,
    version: str,
) -> None:
    print("Cleaning the target directory...")
    shutil.rmtree(BUILD_DIR, ignore_errors=True)
    BUILD_DIR.mkdir(parents=True, exist_ok=True)

    if use_existing_native_libs is None:
        print("Building the native Java library...")
        args = ["cargo", "build", "--package", "sysand-java"]
        if use_release_build:
            args.append("--release")
        execute(args, cwd=BUILD_DIR)

    print("Copying the Java code to the target directory...")
    shutil.copytree(ROOT_DIR / "bindings" / "java" / "java" / "src", BUILD_DIR / "src")

    print("Copying the native libraries to the target directory...")
    native_lib_target_dir = BUILD_DIR / "src" / "main" / "resources"
    native_lib_target_dir.mkdir(parents=True, exist_ok=True)
    native_lib_build_dir_name = "release" if use_release_build else "debug"
    native_lib_build_dir = TARGET_DIR / native_lib_build_dir_name
    if use_existing_native_libs is None:
        # Used when compiling locally, most likely only one of the binaries will
        # be present.
        native_lib_build_path_linux_x64_64 = (
            native_lib_build_dir / "libsysand.so",
            "linux",
            "x86_64",
        )
        native_lib_build_path_linux_aarch64_arm64 = (
            native_lib_build_dir / "libsysand.so",
            "linux",
            "arm64",
        )
        native_lib_build_path_macos_arm64 = (
            native_lib_build_dir / "libsysand.dylib",
            "macos",
            "arm64",
        )
        native_lib_build_path_macos_x86_64 = (
            native_lib_build_dir / "libsysand.dylib",
            "macos",
            "x86_64",
        )
        native_lib_build_path_windows_x64_64 = (
            native_lib_build_dir / "sysand.dll",
            "windows",
            "x86_64",
        )
        native_lib_build_path_windows_aarch64_arm64 = (
            native_lib_build_dir / "sysand.dll",
            "windows",
            "arm64",
        )
    else:
        # Used when compiling in CI, all the binaries are expected to be
        # present.
        native_lib_build_path_linux_x64_64 = (
            use_existing_native_libs / "native-bindings-ubuntu-22.04" / "libsysand.so",
            "linux",
            "x86_64",
        )
        native_lib_build_path_linux_aarch64_arm64 = (
            use_existing_native_libs
            / "native-bindings-ubuntu-24.04-arm"
            / "libsysand.so",
            "linux",
            "arm64",
        )
        native_lib_build_path_macos_arm64 = (
            use_existing_native_libs
            / "native-bindings-macos-latest"
            / "libsysand.dylib",
            "macos",
            "arm64",
        )
        native_lib_build_path_macos_x86_64 = (
            use_existing_native_libs
            / "native-bindings-macos-15-intel"
            / "libsysand.dylib",
            "macos",
            "x86_64",
        )
        native_lib_build_path_windows_x64_64 = (
            use_existing_native_libs / "native-bindings-windows-latest" / "sysand.dll",
            "windows",
            "x86_64",
        )
        native_lib_build_path_windows_aarch64_arm64 = (
            use_existing_native_libs / "native-bindings-windows-11-arm" / "sysand.dll",
            "windows",
            "arm64",
        )
    for native_lib_build_path, os_name, arch_name in [
        native_lib_build_path_linux_x64_64,
        native_lib_build_path_linux_aarch64_arm64,
        native_lib_build_path_macos_arm64,
        native_lib_build_path_macos_x86_64,
        native_lib_build_path_windows_x64_64,
        native_lib_build_path_windows_aarch64_arm64,
    ]:
        if native_lib_build_path.exists():
            target_dir = native_lib_target_dir / f"{os_name}-{arch_name}"
            target_dir.mkdir(parents=True, exist_ok=True)
            target_path = target_dir / native_lib_build_path.name
            print(f"Copying {native_lib_build_path} to {target_path}")
            shutil.copy2(native_lib_build_path, target_path)
        else:
            assert use_existing_native_libs is None, (
                f"Missing native lib: {native_lib_build_path}"
            )

    print(
        "Copying the `pom.xml` template to the target directory and replacing the version..."
    )
    pom_path = ROOT_DIR / "bindings" / "java" / "java" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = BUILD_DIR / "pom.xml"
    full_version = compute_full_version(version, release_jar_version)
    target_pom_data = pom_data.replace("VERSION", full_version)
    target_pom_path.write_text(target_pom_data)

    print("Building the JAR...")
    execute(
        [mvn_executable(), "clean", "install", "compile", "assembly:single", "-U"],
        cwd=BUILD_DIR,
    )


def build_plugin(version: str, release_jar_version: bool) -> None:
    print("Cleaning the target directory for the sysand Maven plugin...")
    shutil.rmtree(PLUGIN_DIR, ignore_errors=True)
    PLUGIN_DIR.mkdir(parents=True, exist_ok=True)

    print(
        "Copying the Java code to the target directory for the sysand Maven plugin..."
    )
    shutil.copytree(
        ROOT_DIR / "bindings" / "java" / "plugin" / "src", PLUGIN_DIR / "src"
    )

    print(
        "Copying the `pom.xml` template to the target directory and replacing the version..."
    )
    pom_path = ROOT_DIR / "bindings" / "java" / "plugin" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = PLUGIN_DIR / "pom.xml"
    full_version = compute_full_version(version, release_jar_version)
    target_pom_data = pom_data.replace("VERSION", full_version)
    target_pom_path.write_text(target_pom_data)

    # # Ensure the sysand artifact is installed in the local Maven repository
    # # before building the plugin, so Maven can resolve the dependency
    # print("Ensuring sysand artifact is installed in local Maven repository...")
    # jar_path = BUILD_DIR / "target" / f"sysand-{full_version}.jar"
    # pom_file_path = BUILD_DIR / "pom.xml"
    # if jar_path.exists() and pom_file_path.exists():
    #     print(f"Installing sysand artifact from {jar_path}...")
    #     execute(
    #         [
    #             mvn_executable(),
    #             "install:install-file",
    #             f"-Dfile={jar_path}",
    #             f"-DpomFile={pom_file_path}",
    #         ],
    #         cwd=BUILD_DIR,
    #     )
    # else:
    #     print(
    #         f"Warning: sysand JAR not found at {jar_path}. "
    #         "Make sure to run 'build' command first, or the plugin build may fail."
    #     )

    print("Building the sysand Maven plugin...")
    execute([mvn_executable(), "-B", "-DskipTests=false", "verify"], cwd=PLUGIN_DIR)


def deploy() -> None:
    # Skip compilation and install to deploy existing artifacts without rebuilding.
    # Note: Maven will still print "[INFO] Building..." as it processes the lifecycle,
    # but actual compilation/rebuilding will be skipped if artifacts are up-to-date.
    args = [
        mvn_executable(),
        "-DskipTests=true",
        "-DskipITs=true",
        "-Dmaven.compiler.skip=true",
        # "-Dinvoker.skip=true",
        "deploy",
    ]
    print("Deploying the Java library to a Maven repository...")
    execute(args, cwd=BUILD_DIR)
    print("Deploying the Sysand Maven plugin to a Maven repository...")
    execute(args, cwd=PLUGIN_DIR)


def test_deployed(version: str, release_jar_version: bool) -> None:
    print("Testing the deployed Java library...")
    full_version = compute_full_version(version, release_jar_version)

    print("Copying the deploy test code to the target directory...")
    shutil.rmtree(DEPLOY_TEST_DIR, ignore_errors=True)
    shutil.copytree(
        ROOT_DIR / "bindings" / "java" / "java-deploy-test", DEPLOY_TEST_DIR
    )

    print("Replacing com.sensmetry.sysand dependency version in deploy test pom.xml")
    pom_path = ROOT_DIR / "bindings" / "java" / "java-deploy-test" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = DEPLOY_TEST_DIR / "pom.xml"
    target_pom_data = pom_data.replace("VERSION", full_version)
    target_pom_path.write_text(target_pom_data)

    print("Testing the deployed Java library...")
    execute([mvn_executable(), "test"], cwd=DEPLOY_TEST_DIR)


def test(version: str, release_jar_version: bool) -> None:
    print("Looking for Java library...")
    full_version = compute_full_version(version, release_jar_version)
    jar_path = BUILD_DIR / "target" / f"sysand-{full_version}.jar"
    if not jar_path.exists():
        raise FileNotFoundError(f"JAR file not found: {jar_path}")
    else:
        print("Installing the Java library...")
        execute(
            [mvn_executable(), "install:install-file", f"-Dfile={jar_path}"],
            cwd=BUILD_DIR,
        )

    print("Copying the test code to the target directory...")
    shutil.rmtree(TEST_DIR, ignore_errors=True)
    shutil.copytree(ROOT_DIR / "bindings" / "java" / "java-test", TEST_DIR)

    print("Replacing com.sensmetry.sysand dependency version in test pom.xml")
    pom_path = ROOT_DIR / "bindings" / "java" / "java-test" / "pom.xml"
    pom_data = pom_path.read_text()
    target_pom_path = TEST_DIR / "pom.xml"
    target_pom_data = pom_data.replace("VERSION", full_version)
    target_pom_path.write_text(target_pom_data)

    print("Testing the Java library...")
    execute([mvn_executable(), "test"], cwd=TEST_DIR)


def main() -> None:
    args = parse_args()
    release_jar_version = args.release_jar_version
    # Check environment variable for release-jar-version flag
    TRUE_CONSTANTS = ("1", "true")
    if os.getenv("JAVA_BUILDER_RELEASE_JAR_VERSION", "").lower() in TRUE_CONSTANTS:
        release_jar_version = True
    print("ROOT_DIR:", ROOT_DIR)
    print("BUILD_DIR:", BUILD_DIR)
    print("TEST_DIR:", TEST_DIR)
    version = parse_version()
    print("Version:", version)
    if args.command == "build":
        build(
            args.use_release_build,
            args.use_existing_native_libs,
            release_jar_version,
            version,
        )
    elif args.command == "build-plugin":
        build_plugin(version, release_jar_version)
    elif args.command == "test":
        test(version, release_jar_version)
    elif args.command == "test-deployed":
        test_deployed(version, release_jar_version)
    elif args.command == "deploy":
        deploy()
    elif args.command == "create-version-file":
        create_version_file(version)


if __name__ == "__main__":
    main()
