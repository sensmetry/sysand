#!/bin/sh

# Sysand installer for Linux and macOS.
#
# This script is intentionally small and direct so it can be inspected before
# running. It downloads a release archive, extracts the sysand binary, and
# atomically replaces the installed binary.

set -eu

repo="sensmetry/sysand"
version="latest"
install_dir=""
modify_path="true"

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

  --no-modify-path      Do not add the install directory to PATH.

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
      [ -n "$2" ] || fail "--install-dir requires a non-empty value"
      install_dir="$2"
      shift 2
      ;;
    --no-modify-path)
      modify_path="false"
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

if [ -n "${SYSAND_NO_MODIFY_PATH:-}" ]; then
  modify_path="false"
fi

default_install_dir() {
  if [ -n "${HOME:-}" ]; then
    printf "%s\n" "${HOME}/.local/bin"
  else
    fail "could not determine an install directory; set HOME or use --install-dir"
  fi
}

[ -n "$install_dir" ] || install_dir="$(default_install_dir)"

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

install_binary() {
  mkdir -p "$install_dir"
  installed_tmp="$(mktemp "${install_dir}/.sysand.XXXXXX")" \
    || fail "failed to create temporary install file in \`${install_dir}\`"
  if ! cp "$extracted_binary" "$installed_tmp"; then
    rm -f "$installed_tmp"
    fail "failed to copy sysand into \`${install_dir}\`"
  fi
  if ! chmod 755 "$installed_tmp"; then
    rm -f "$installed_tmp"
    fail "failed to make \`${installed_tmp}\` executable"
  fi
  if ! mv -f "$installed_tmp" "${install_dir}/sysand"; then
    rm -f "$installed_tmp"
    fail "failed to replace \`${install_dir}/sysand\`"
  fi
}

shell_quote() {
  printf "%s\n" "$1" | sed "s/'/'\\\\''/g; 1s/^/'/; \$s/\$/'/"
}

add_to_github_path() {
  [ -n "${GITHUB_PATH:-}" ] || return 1
  if [ -f "$GITHUB_PATH" ] && grep -Fx -- "$install_dir" "$GITHUB_PATH" >/dev/null 2>&1; then
    return 0
  fi
  printf "%s\n" "$install_dir" >> "$GITHUB_PATH"
  return 0
}

profile_path() {
  [ -n "${HOME:-}" ] || fail "could not update shell profile because HOME is not set; rerun with --no-modify-path"

  case "${SHELL:-}" in
    */zsh)
      printf "%s\n" "${HOME}/.zprofile"
      ;;
    */bash)
      if [ -f "${HOME}/.bashrc" ]; then
        printf "%s\n" "${HOME}/.bashrc"
      else
        printf "%s\n" "${HOME}/.profile"
      fi
      ;;
    *)
      printf "%s\n" "${HOME}/.profile"
      ;;
  esac
}

add_to_shell_profile() {
  profile="$(profile_path)"
  quoted_install_dir="$(shell_quote "$install_dir")"

  if [ -f "$profile" ] && grep -F "# >>> sysand installer >>>" "$profile" >/dev/null 2>&1; then
    return 0
  fi

  {
    printf "\n"
    printf "# >>> sysand installer >>>\n"
    printf "sysand_bin_dir=%s\n" "$quoted_install_dir"
    printf "case \":\${PATH}:\" in\n"
    printf "  *\":\${sysand_bin_dir}:\"*) ;;\n"
    printf "  *) export PATH=\"\${sysand_bin_dir}:\${PATH}\" ;;\n"
    printf "esac\n"
    printf "# <<< sysand installer <<<\n"
  } >> "$profile" || fail "failed to update shell profile \`${profile}\`"

  echo
  echo "Added ${install_dir} to PATH in ${profile}."
}

modify_path_if_needed() {
  [ "$modify_path" = "true" ] || return 0

  if add_to_github_path; then
    echo "Added ${install_dir} to GITHUB_PATH."
    return 0
  fi

  case ":${PATH:-}:" in
    *":${install_dir}:"*)
      return 0
      ;;
  esac

  add_to_shell_profile
}

print_next_steps() {
  [ "${GITHUB_ACTIONS:-}" = "true" ] && return 0

  echo
  if [ "$modify_path" = "true" ]; then
    echo "Open a new terminal to run sysand by name."
  else
    echo "PATH was not modified. Add ${install_dir} to PATH to run sysand by name."
  fi
  echo
  echo "For this already-open shell, run:"
  echo "  export PATH=\"${install_dir}:\$PATH\""
}

need_cmd uname
need_cmd mktemp
need_cmd tar
need_cmd grep
need_cmd sed

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
modify_path_if_needed
print_next_steps
