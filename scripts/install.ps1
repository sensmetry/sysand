# Sysand installer for Windows.
#
# This script is intentionally direct so it can be inspected before running.
# It downloads a release archive, extracts sysand.exe, installs it atomically
# into %LOCALAPPDATA%\Programs\Sensmetry\Sysand\bin (replacing any existing
# installation), and ensures that directory is on the user PATH.
#
# Configuration is via environment variables:
#   SYSAND_VERSION           Release version to install, for example 0.1.0 or
#                            v0.1.0-rc.1. Default: latest non-prerelease.
#   SYSAND_INSTALL_BASE_URL  For local tests. It should point at a directory
#                            containing the release asset files.
#
# Requires Windows 10 or newer.

if ($args.Count -gt 0) {
    if ($args[0] -in @("-h", "--help", "-help", "/?")) {
        @'
Sysand installer for Windows

Usage:
  .\install.ps1 [-Help]

Installs sysand.exe to %LOCALAPPDATA%\Programs\Sensmetry\Sysand\bin,
replacing any existing installation, and ensures that directory is on the
user PATH.

Configuration via environment variables:
  SYSAND_VERSION   Release version to install, for example 0.1.0 or
                   0.1.0-rc.1. A leading "v" is also accepted.
                   Default: latest non-prerelease.

Uninstall by deleting %LOCALAPPDATA%\Programs\Sensmetry\Sysand and removing
the install directory from your user PATH.
'@
        return
    }
    throw "error: this installer takes no arguments besides -Help; set SYSAND_VERSION=<version> to pick a version. The install directory is fixed to %LOCALAPPDATA%\Programs\Sensmetry\Sysand\bin."
}

# The body runs in its own scope so that, when invoked via "irm ... | iex",
# preference and helper variables do not leak into the calling session.
& {

$ErrorActionPreference = "Stop"

function Fail {
    param([string]$Message)
    throw "error: $Message"
}

# Windows 10 also implies the PowerShell (5.1+) and .NET (TLS 1.2 capable)
# versions this script needs.
if ([Environment]::OSVersion.Version.Major -lt 10) {
    Fail "sysand requires Windows 10 or newer (found $([Environment]::OSVersion.Version))"
}

$Version = if ($env:SYSAND_VERSION) { $env:SYSAND_VERSION } else { "latest" }

# Keep the version value narrow so it cannot accidentally form a surprising URL.
if ($Version -cnotmatch '^[a-z0-9.-]+$') {
    Fail "SYSAND_VERSION may only contain lowercase letters, numbers, dots, and dashes"
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
    default { Fail "unsupported architecture: $ProcessorArchitecture" }
}

# Build the release asset URL. "latest" means GitHub's latest non-prerelease.
$Repo = "sensmetry/sysand"
$Asset = "sysand-windows-$Arch.zip"

if ($env:SYSAND_INSTALL_BASE_URL) {
    $BaseUrl = $env:SYSAND_INSTALL_BASE_URL.TrimEnd("/")
    $DownloadUrl = "$BaseUrl/$Asset"
} elseif ($Tag -eq "latest") {
    $DownloadUrl = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $DownloadUrl = "https://github.com/$Repo/releases/download/$Tag/$Asset"
}

$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\Sensmetry\Sysand\bin"
$LegacyDir = Join-Path $env:LOCALAPPDATA "Programs\Sysand"

function Test-SamePathEntry {
    param([string]$A, [string]$B)
    return $A.TrimEnd("\") -eq $B.TrimEnd("\")
}

# Install the staged exe with renames so the destination never holds a partial
# binary. A running sysand.exe can be renamed but not deleted or overwritten,
# so an update moves it aside first and cleans it up on the next run.
function Install-Binary {
    param([string]$ExtractedBinary)

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    $Dest = Join-Path $InstallDir "sysand.exe"

    Get-ChildItem -Path $InstallDir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "sysand.exe.old.*" -or $_.Name -like "sysand.exe.new.*" } |
        Remove-Item -Force -ErrorAction SilentlyContinue

    $Staging = Join-Path $InstallDir ("sysand.exe.new." + [System.IO.Path]::GetRandomFileName())
    Copy-Item -Path $ExtractedBinary -Destination $Staging

    $Old = $null
    if (Test-Path -Path $Dest) {
        $Old = Join-Path $InstallDir ("sysand.exe.old." + [System.IO.Path]::GetRandomFileName())
        Move-Item -Path $Dest -Destination $Old
    }
    try {
        Move-Item -Path $Staging -Destination $Dest
    } catch {
        if ($Old) { Move-Item -Path $Old -Destination $Dest }
        throw
    }
    if ($Old) { Remove-Item -Path $Old -Force -ErrorAction SilentlyContinue }
}

# Earlier installer versions placed sysand.exe directly in Programs\Sysand;
# leaving it would keep shadowing or duplicating the new installation.
function Remove-LegacyInstall {
    $LegacyBinary = Join-Path $LegacyDir "sysand.exe"
    if (Test-Path -Path $LegacyBinary) {
        try {
            Remove-Item -Path $LegacyBinary -Force
            Write-Host "Removed previous installation at $LegacyBinary"
        } catch {
            Write-Warning "could not remove previous installation at $LegacyBinary; delete it manually"
        }
    }
}

# Add the install directory to the persisted user PATH, editing the registry
# value directly so a REG_EXPAND_SZ PATH keeps its kind and its unexpanded
# %VAR% entries (which [Environment]::SetEnvironmentVariable would clobber).
# Also drops the legacy Programs\Sysand entry. Best effort: warns on failure.
# Returns $true when the install directory was already persisted.
function Update-UserPath {
    $EnvKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    try {
        $HasPath = $EnvKey.GetValueNames() -contains "Path"
        $RawPath = ""
        $Kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
        if ($HasPath) {
            $RawPath = $EnvKey.GetValue(
                "Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $Kind = $EnvKey.GetValueKind("Path")
        }

        $Entries = @($RawPath -split ";" | Where-Object { $_ -ne "" })
        $Kept = @($Entries | Where-Object { -not (Test-SamePathEntry $_ $LegacyDir) })
        $AlreadyPresent = @($Kept | Where-Object { Test-SamePathEntry $_ $InstallDir }).Count -gt 0

        if ($AlreadyPresent -and $Kept.Count -eq $Entries.Count) {
            return $true
        }
        if (-not $AlreadyPresent) {
            $Kept += $InstallDir
        }
        $EnvKey.SetValue("Path", ($Kept -join ";"), $Kind)
        if ($AlreadyPresent) {
            Write-Host "Removed legacy $LegacyDir from your user PATH"
        } else {
            Write-Host "Added $InstallDir to your user PATH (takes effect after you sign out and back in)"
        }
        return $AlreadyPresent
    } finally {
        $EnvKey.Dispose()
    }
}

function Set-SysandOnPath {
    if ($env:GITHUB_PATH) {
        Add-Content -Path $env:GITHUB_PATH -Value $InstallDir
        Write-Host "Added $InstallDir to GITHUB_PATH"
        return
    }

    # In CI the job shell is the only session; a persisted PATH edit would be
    # a pointless write to an ephemeral environment.
    $AlreadyPersisted = $false
    if ($env:CI) {
        Write-Host "CI environment detected; not updating the user PATH"
    } else {
        try {
            $AlreadyPersisted = Update-UserPath
        } catch {
            Write-Warning "could not update your user PATH ($_); add $InstallDir to it manually"
        }
    }

    # An already persisted PATH means an earlier run printed the session
    # instructions; repeating them is noise.
    $OnLivePath = @($env:Path -split ";" | Where-Object { Test-SamePathEntry $_ $InstallDir }).Count -gt 0
    if (-not $OnLivePath -and -not $AlreadyPersisted) {
        Write-Host ""
        Write-Host "To use sysand in this session:"
        Write-Host "  PowerShell:  `$env:Path = `"$InstallDir;`" + `$env:Path"
        Write-Host "  cmd.exe:     set PATH=$InstallDir;%PATH%"
    }
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
$Archive = Join-Path $TempDir $Asset
$ExtractDir = Join-Path $TempDir "extract"

try {
    New-Item -ItemType Directory -Path $TempDir, $ExtractDir -Force | Out-Null

    Write-Host "Downloading $DownloadUrl"
    try {
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $Archive -UseBasicParsing
    } catch {
        Fail "failed to download $Asset from $DownloadUrl ($_)"
    }

    Write-Host "Extracting $Asset"
    Expand-Archive -Path $Archive -DestinationPath $ExtractDir -Force

    $ExtractedBinary = Join-Path $ExtractDir "sysand.exe"
    if (-not (Test-Path -Path $ExtractedBinary -PathType Leaf)) {
        Fail "archive did not contain a sysand.exe binary at its root"
    }

    Write-Host "Installing sysand to $(Join-Path $InstallDir "sysand.exe")"
    Install-Binary -ExtractedBinary $ExtractedBinary

    $InstalledVersion = & (Join-Path $InstallDir "sysand.exe") --version
    Write-Host "Installed $InstalledVersion"

    Remove-LegacyInstall
    Set-SysandOnPath
} finally {
    if (Test-Path $TempDir) {
        Remove-Item -Path $TempDir -Recurse -Force
    }
}

}
