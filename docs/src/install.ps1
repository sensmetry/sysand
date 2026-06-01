# Sysand installer for Windows.
#
# This script is intentionally small and direct so it can be inspected before
# running. It downloads a release archive, extracts sysand.exe, and copies it
# into an install directory.

param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\Sysand",
    [switch]$Help
)

$ErrorActionPreference = "Stop"

function Show-Usage {
    @"
Sysand installer for Windows

Usage:
  .\install.ps1 [-Version <version>] [-InstallDir <path>] [-Help]

Options:
  -Version <version>    Install a specific release version, for example 0.1.0
                        or 0.1.0-rc.1. A leading "v" is also accepted.
                        Default: latest non-prerelease.

  -InstallDir <path>    Install into a specific directory.
                        Default: `$env:LOCALAPPDATA\Programs\Sysand

  -Help                 Print this help text.
"@
}

function Fail {
    param([string]$Message)
    Write-Error $Message
    exit 1
}

if ($Help) {
    Show-Usage
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    Fail "Version must not be empty."
}

if ($Version -notmatch '^[A-Za-z0-9.-]+$') {
    Fail "Version may only contain letters, numbers, dots, and dashes."
}

# Accept versions with or without a leading "v". GitHub release tags use "v".
if ($Version -eq "latest") {
    $Tag = "latest"
} elseif ($Version.StartsWith("v")) {
    $Tag = $Version
} else {
    $Tag = "v$Version"
}

# Detect the CPU architecture name used by Sysand release assets.
$ProcessorArchitecture = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}

switch ($ProcessorArchitecture) {
    "AMD64" { $Arch = "x86_64" }
    "ARM64" { $Arch = "arm64" }
    default { Fail "Unsupported architecture: $ProcessorArchitecture" }
}

# Build the release asset URL. "latest" means GitHub's latest non-prerelease.
$Repo = "sensmetry/sysand"
$Asset = "sysand-windows-$Arch.zip"

# SYSAND_INSTALL_BASE_URL is for local tests. It should point at a directory
# containing the release asset files.
if ($env:SYSAND_INSTALL_BASE_URL) {
    $BaseUrl = $env:SYSAND_INSTALL_BASE_URL.TrimEnd("/")
    $DownloadUrl = "$BaseUrl/$Asset"
} elseif ($Tag -eq "latest") {
    $DownloadUrl = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $DownloadUrl = "https://github.com/$Repo/releases/download/$Tag/$Asset"
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
$Archive = Join-Path $TempDir $Asset
$ExtractDir = Join-Path $TempDir "extract"

try {
    New-Item -ItemType Directory -Path $TempDir, $ExtractDir -Force | Out-Null

    Write-Host "Downloading $DownloadUrl"
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $Archive

    Write-Host "Extracting $Asset"
    Expand-Archive -Path $Archive -DestinationPath $ExtractDir -Force

    $ExtractedBinary = Join-Path $ExtractDir "sysand.exe"
    if (-not (Test-Path -Path $ExtractedBinary -PathType Leaf)) {
        Fail "Archive did not contain a sysand.exe binary at its root."
    }

    $InstalledBinary = Join-Path $InstallDir "sysand.exe"

    Write-Host "Installing sysand to $InstalledBinary"
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path $ExtractedBinary -Destination $InstalledBinary -Force

    $InstalledVersion = & $InstalledBinary --version
    Write-Host "Installed $InstalledVersion"

    $PathEntries = [Environment]::GetEnvironmentVariable("Path", "User") -split ";"
    if ($PathEntries -notcontains $InstallDir) {
        Write-Host ""
        Write-Host "Note: $InstallDir is not currently on your user PATH."
        Write-Host "Add it to PATH to run sysand by name from a new terminal."
    }
} finally {
    if (Test-Path $TempDir) {
        Remove-Item -Path $TempDir -Recurse -Force
    }
}
