#!/bin/sh

# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

# Sysand installer for Linux and macOS.
#
# This script is intentionally small and direct so it can be inspected before
# running. It downloads a release archive, extracts the sysand binary, and copies
# it into an install directory.

set -eu

repo="sensmetry/sysand"
version="latest"
install_dir="${HOME}/.local/bin"
custom_install_dir="false"
system_install="false"

print_usage() {
  cat <<'EOF'
Sysand installer for Linux and macOS

Usage:
  sh install.sh [options]

Options:
  --version <version>   Install a specific release version, for example 0.1.0
                        or 0.1.0-rc.1. A leading "v" is also accepted.
                        Default: latest non-prerelease.

  --install-dir <path>  Install into a specific directory.
                        Default: $HOME/.local/bin

  --system-install      Install into /usr/local/bin.
                        If the current user is not root, the install step uses
                        sudo.

  -h, --help            Print this help text.
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || fail "--install-dir requires a value"
      [ "$system_install" = "false" ] || fail "--install-dir cannot be used with --system-install"
      install_dir="$2"
      custom_install_dir="true"
      shift 2
      ;;
    --system-install)
      [ "$custom_install_dir" = "false" ] || fail "--system-install cannot be used with --install-dir"
      install_dir="/usr/local/bin"
      system_install="true"
      shift
      ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

# Accept versions with or without a leading "v". GitHub release tags use "v".
normalize_version() {
  case "$version" in
    latest)
      tag="latest"
      ;;
    v*)
      tag="$version"
      ;;
    *)
      tag="v$version"
      ;;
  esac
}

# Keep the version value narrow so it cannot accidentally form a surprising URL.
validate_version() {
  [ -n "$version" ] || fail "version must not be empty"

  case "$version" in
    *[!abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-]*)
      fail "version may only contain letters, numbers, dots, and dashes"
      ;;
  esac
}

# Detect the operating system name used by Sysand release assets.
detect_os() {
  case "$(uname -s)" in
    Linux)
      os="linux"
      ;;
    Darwin)
      os="macos"
      ;;
    *)
      fail "unsupported operating system: $(uname -s)"
      ;;
  esac
}

# Detect the CPU architecture name used by Sysand release assets.
detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)
      arch="x86_64"
      ;;
    arm64|aarch64)
      arch="arm64"
      ;;
    *)
      fail "unsupported architecture: $(uname -m)"
      ;;
  esac
}

# Build the release asset URL. "latest" means GitHub's latest non-prerelease.
build_download_url() {
  asset="sysand-${os}-${arch}.tar.gz"

  # SYSAND_INSTALL_BASE_URL is for local tests. It should point at a directory
  # containing the release asset files.
  if [ -n "${SYSAND_INSTALL_BASE_URL:-}" ]; then
    base_url="${SYSAND_INSTALL_BASE_URL%/}"
    download_url="${base_url}/${asset}"
  elif [ "$tag" = "latest" ]; then
    download_url="https://github.com/${repo}/releases/latest/download/${asset}"
  else
    download_url="https://github.com/${repo}/releases/download/${tag}/${asset}"
  fi
}

download_file() {
  url="$1"
  output="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$output"
  else
    fail "curl or wget is required"
  fi
}

run_system_install() {
  if [ "$(id -u)" = "0" ]; then
    "$@"
  else
    sudo "$@"
  fi
}

install_binary() {
  if [ "$system_install" = "true" ]; then
    run_system_install mkdir -p "$install_dir"
    run_system_install cp "$extracted_binary" "${install_dir}/sysand"
  else
    mkdir -p "$install_dir"
    cp "$extracted_binary" "${install_dir}/sysand"
  fi
}

print_path_note() {
  case ":${PATH:-}:" in
    *":${install_dir}:"*)
      ;;
    *)
      echo
      echo "Note: ${install_dir} is not currently on PATH."
      echo "Add it to your shell profile to run sysand by name, for example:"
      echo "  export PATH=\"${install_dir}:\$PATH\""
      ;;
  esac
}

need_cmd uname
need_cmd mktemp
need_cmd tar

validate_version
normalize_version
detect_os
detect_arch
build_download_url

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

archive="${tmp_dir}/${asset}"
extract_dir="${tmp_dir}/extract"
mkdir -p "$extract_dir"

echo "Downloading ${download_url}"
download_file "$download_url" "$archive"

echo "Extracting ${asset}"
tar -xzf "$archive" -C "$extract_dir"

extracted_binary="${extract_dir}/sysand"
[ -f "$extracted_binary" ] || fail "archive did not contain a sysand binary at its root"

echo "Installing sysand to ${install_dir}/sysand"
install_binary

echo "Installed $("${install_dir}/sysand" --version)"
print_path_note
