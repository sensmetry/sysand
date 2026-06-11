#!/bin/sh

# Sysand installer for Linux and macOS.
#
# This script is intentionally direct so it can be inspected before running.
# It downloads a release archive, extracts the sysand binary, installs it
# atomically into $HOME/.local/bin (replacing any existing installation), and
# ensures that directory is on PATH.
#
# Configuration is via environment variables:
#   SYSAND_VERSION           Release version to install, for example 0.1.2 or
#                            v0.1.2-rc.1. Must be 0.1.2 or later.
#                            Default: latest non-prerelease.
#   SYSAND_INSTALL_BASE_URL  For local tests. It should point at a directory
#                            containing the release asset files.

set -eu

# Everything happens in functions and main is the last line of the script, so
# a partially downloaded "curl | sh" defines functions but executes nothing.

print_usage() {
  cat <<'EOF'
Sysand installer for Linux and macOS

Usage:
  sh install.sh [-h|--help]

Installs the sysand binary to $HOME/.local/bin, replacing any existing
installation, and ensures that directory is on PATH.

Configuration via environment variables:
  SYSAND_VERSION   Release version to install, for example 0.1.2 or
                   0.1.2-dev.1. A leading "v" is also accepted. Must be
                   0.1.2 or later. Default: latest non-prerelease.

Uninstall by deleting $HOME/.local/bin/sysand and removing the PATH line the
installer added to your shell profile.
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h|--help)
        print_usage
        exit 0
        ;;
      --version)
        fail "--version was removed; set SYSAND_VERSION=<version> instead"
        ;;
      --install-dir|--system-install)
        fail "$1 was removed; sysand now always installs to \$HOME/.local/bin"
        ;;
      *)
        fail "unknown option: $1 (configuration is via environment variables, see --help)"
        ;;
    esac
  done
}

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
  case "$version" in
    *[!abcdefghijklmnopqrstuvwxyz0123456789.-]*)
      fail "SYSAND_VERSION may only contain lowercase letters, numbers, dots, and dashes"
      ;;
  esac
}

# Releases before 0.1.2 do not publish all the assets this installer expects
# (for example the musl builds), so only 0.1.2 or later is supported.
check_version_supported() {
  [ "$version" != "latest" ] || return 0

  release="${version#v}"
  release="${release%%-*}"
  case "$release" in
    *.*.*) ;;
    *)
      fail "could not parse SYSAND_VERSION \`${version}\` as major.minor.patch"
      ;;
  esac
  major="${release%%.*}"
  rest="${release#*.}"
  minor="${rest%%.*}"
  rest="${rest#*.}"
  patch="${rest%%.*}"
  case "${major}~${minor}~${patch}" in
    *[!0-9~]*|*~~*)
      fail "could not parse SYSAND_VERSION \`${version}\` as major.minor.patch"
      ;;
  esac

  if [ "$major" -eq 0 ] &&
    { [ "$minor" -lt 1 ] || { [ "$minor" -eq 1 ] && [ "$patch" -lt 2 ]; }; }; then
    fail "this installer only supports sysand 0.1.2 or later (requested ${version})"
  fi
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
      # A shell under Rosetta 2 reports x86_64 on Apple Silicon; prefer the
      # native arm64 build.
      if [ "$os" = "macos" ] &&
        [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
        arch="arm64"
      fi
      ;;
    arm64|aarch64)
      arch="arm64"
      ;;
    *)
      fail "unsupported architecture: $(uname -m)"
      ;;
  esac
}

# The macOS release binaries require macOS 11 (Big Sur) on arm64 and
# macOS 10.12 (Sierra) on x86_64, the Rust toolchain minimums.
check_macos_version() {
  macos_version="$(sw_vers -productVersion)"
  macos_major="${macos_version%%.*}"
  macos_minor="${macos_version#*.}"
  macos_minor="${macos_minor%%[!0-9]*}"
  case "$macos_version" in
    *.*) ;;
    *) macos_minor="0" ;;
  esac
  case "$macos_major" in
    ''|*[!0-9]*)
      fail "could not parse macOS version \`${macos_version}\`"
      ;;
  esac
  [ -n "$macos_minor" ] || macos_minor="0"

  if [ "$arch" = "arm64" ]; then
    [ "$macos_major" -ge 11 ] ||
      fail "sysand requires macOS 11 or newer on arm64 (found ${macos_version})"
  elif [ "$macos_major" -lt 10 ] ||
    { [ "$macos_major" -eq 10 ] && [ "$macos_minor" -lt 12 ]; }; then
    fail "sysand requires macOS 10.12 or newer on x86_64 (found ${macos_version})"
  fi
}

# Pick the gnu build when glibc 2.35+ is available, the fully static musl
# build otherwise (older glibc, musl, or undetectable).
detect_libc() {
  glibc_version=""
  if glibc_output="$(getconf GNU_LIBC_VERSION 2>/dev/null)"; then
    # getconf prints "glibc 2.39". On musl this query fails.
    glibc_version="${glibc_output#glibc }"
  else
    # musl's ldd prints "musl libc (...)" to stderr; glibc's prints a first
    # line ending in the version.
    ldd_line="$(ldd --version 2>&1 | head -n 1 || true)"
    case "$ldd_line" in
      *musl*) ;;
      *)
        glibc_version="$(echo "$ldd_line" | grep -o '[0-9][0-9.]*$' || true)"
        ;;
    esac
  fi

  glibc_major="${glibc_version%%.*}"
  glibc_minor="${glibc_version#*.}"
  glibc_minor="${glibc_minor%%[!0-9]*}"
  case "$glibc_version" in
    *.*) ;;
    *) glibc_major="" ;;
  esac
  case "$glibc_major" in
    ''|*[!0-9]*) glibc_major="" ;;
  esac

  if [ -n "$glibc_major" ] && [ -n "$glibc_minor" ] &&
    { [ "$glibc_major" -gt 2 ] ||
      { [ "$glibc_major" -eq 2 ] && [ "$glibc_minor" -ge 35 ]; }; }; then
    libc="gnu"
    echo "Detected glibc ${glibc_version}; using the gnu build"
  elif [ -n "$glibc_major" ]; then
    libc="musl"
    echo "Detected glibc ${glibc_version} (older than 2.35); using the static musl build"
  else
    libc="musl"
    echo "Could not detect glibc; using the static musl build"
  fi
}

# Build the release asset URL. "latest" means GitHub's latest non-prerelease.
build_download_url() {
  if [ "$os" = "linux" ]; then
    asset="sysand-${os}-${arch}-${libc}.tar.gz"
  else
    asset="sysand-${os}-${arch}.tar.gz"
  fi

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
    curl -fsSL "$url" -o "$output" || fail "failed to download \`${asset}\` from \`${url}\`"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$output" || fail "failed to download \`${asset}\` from \`${url}\`"
  else
    fail "curl or wget is required"
  fi
}

# Stage inside the destination directory so the final move is an atomic
# same-filesystem rename; the destination never holds a partial binary.
install_binary() {
  mkdir -p "$install_dir"
  rm -f "${install_dir}/.sysand."*
  staging="$(mktemp "${install_dir}/.sysand.XXXXXX")"
  cp "$extracted_binary" "$staging"
  chmod 755 "$staging"
  mv -f "$staging" "${install_dir}/sysand"
  staging=""
}

# Append the install directory to the profile of the user's login shell so
# future sessions find sysand. Best effort: warns instead of failing.
persist_path() {
  shell_name="${SHELL:-}"
  shell_name="${shell_name##*/}"
  case "$shell_name" in
    zsh)
      profile="${ZDOTDIR:-$HOME}/.zshrc"
      ;;
    bash)
      if [ "$os" = "macos" ]; then
        profile="${HOME}/.bash_profile"
      else
        profile="${HOME}/.bashrc"
      fi
      ;;
    *)
      echo "warning: unsupported shell \`${SHELL:-}\`; add this line to your shell profile yourself:" >&2
      echo "  ${path_line}" >&2
      return 0
      ;;
  esac

  if [ -f "$profile" ] && grep -qF "$path_line" "$profile"; then
    path_already_persisted="true"
    return 0
  fi

  if {
    echo ""
    echo "# Added by the sysand installer"
    echo "$path_line"
  } >> "$profile" 2>/dev/null; then
    echo "Added ${install_dir} to PATH in ${profile} (takes effect in new shells)"
  else
    echo "warning: could not update ${profile}; add this line to it yourself:" >&2
    echo "  ${path_line}" >&2
  fi
}

configure_path() {
  if [ -n "${GITHUB_PATH:-}" ]; then
    echo "$install_dir" >> "$GITHUB_PATH"
    echo "Added ${install_dir} to GITHUB_PATH"
    return 0
  fi

  case ":${PATH:-}:" in
    *":${install_dir}:"*)
      return 0
      ;;
  esac

  # In CI the job shell is the only session; a profile edit would be a
  # pointless write to an ephemeral environment.
  path_already_persisted="false"
  if [ -n "${CI:-}" ]; then
    echo "CI environment detected; not updating any shell profile"
  else
    persist_path
  fi

  # A profile that already has the line means an earlier run printed the
  # session instructions; repeating them is noise.
  if [ "$path_already_persisted" = "false" ]; then
    echo "To use sysand in this shell, run:"
    echo "  ${path_line}"
  fi
}

# An older sysand earlier on PATH (for example a /usr/local/bin system
# install) would keep being picked up instead of this one.
warn_shadowed() {
  case ":${PATH:-}:" in
    *":${install_dir}:"*) ;;
    *)
      return 0
      ;;
  esac
  resolved="$(command -v sysand 2>/dev/null || true)"
  if [ -n "$resolved" ] && [ "$resolved" != "${install_dir}/sysand" ]; then
    echo "warning: \`${resolved}\` comes before \`${install_dir}/sysand\` on PATH and will shadow it" >&2
  fi
}

main() {
  parse_args "$@"

  need_cmd uname
  need_cmd mktemp
  need_cmd tar
  [ -n "${HOME:-}" ] || fail "HOME must be set"

  repo="sensmetry/sysand"
  version="${SYSAND_VERSION:-latest}"
  install_dir="${HOME}/.local/bin"
  # Written verbatim to shell profiles; $HOME must stay unexpanded.
  # shellcheck disable=SC2016
  path_line='export PATH="$HOME/.local/bin:$PATH"'

  validate_version
  check_version_supported
  normalize_version
  detect_os
  detect_arch
  if [ "$os" = "macos" ]; then
    check_macos_version
  fi
  if [ "$os" = "linux" ]; then
    detect_libc
  fi
  build_download_url

  tmp_dir="$(mktemp -d)"
  staging=""
  trap 'rm -rf "$tmp_dir"; if [ -n "$staging" ]; then rm -f "$staging"; fi' EXIT

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
  configure_path
  warn_shadowed
}

main "$@"
