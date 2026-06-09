# Sysand installer for Windows.
#
# This script is intentionally small and direct so it can be inspected before
# running. It downloads a release archive, extracts sysand.exe, and replaces the
# installed binary.

param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\Sensmetry\Sysand\bin",
    [switch]$NoModifyPath,
    [switch]$Help
)

$ErrorActionPreference = "Stop"

function Show-Usage {
    @"
Sysand installer for Windows

Usage:
  .\install.ps1 [-Version <version>] [-InstallDir <path>] [-NoModifyPath] [-Help]

Options:
  -Version <version>    Install a specific release version, for example 0.1.0
                        or 0.1.0-rc.1. A leading "v" is also accepted.
                        Default: latest non-prerelease.

  -InstallDir <path>    Install into a specific directory.
                        Default: `$env:LOCALAPPDATA\Programs\Sensmetry\Sysand\bin

  -NoModifyPath         Do not add the install directory to PATH.

  -Help                 Print this help text.
"@
}

function Fail {
    param([string]$Message)
    Write-Error $Message
    exit 1
}

function Initialize-Environment {
    if ($PSVersionTable.PSVersion.Major -lt 5) {
        Fail @"
PowerShell 5 or later is required to install Sysand.
Detected PowerShell $($PSVersionTable.PSVersion).
"@
    }

    if ([System.Enum]::GetNames([System.Net.SecurityProtocolType]) -notcontains "Tls12") {
        Fail @"
Installing Sysand requires TLS 1.2 support from .NET Framework 4.5 or later.
Install a newer .NET Framework before running this installer.
"@
    }

    [System.Net.ServicePointManager]::SecurityProtocol = `
        [System.Net.ServicePointManager]::SecurityProtocol -bor `
        [System.Net.SecurityProtocolType]::Tls12
}

function Add-CiPath {
    param([string]$LiteralPath)
    if ($env:GITHUB_PATH) {
        if ((Test-Path $env:GITHUB_PATH) -and (Get-Content -Path $env:GITHUB_PATH | Where-Object { $_ -eq $LiteralPath })) {
            return $true
        }
        Write-Output $LiteralPath | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
        return $true
    }
    return $false
}

function Add-UserPath {
    param([string]$LiteralPath)

    $RegistryPath = "registry::HKEY_CURRENT_USER\Environment"
    $CurrentPath = (Get-Item -LiteralPath $RegistryPath).GetValue("Path", "", "DoNotExpandEnvironmentNames")
    $CurrentDirectories = @($CurrentPath -split ";" | Where-Object { $_ -ne "" })
    foreach ($CurrentDirectory in $CurrentDirectories) {
        if ([StringComparer]::OrdinalIgnoreCase.Equals($CurrentDirectory, $LiteralPath)) {
            return $false
        }
    }

    if ($CurrentDirectories.Count -eq 0) {
        $NewPath = $LiteralPath
    } else {
        $NewPath = ($CurrentDirectories + $LiteralPath) -join ";"
    }

    if ([string]::IsNullOrEmpty($NewPath)) {
        return $false
    }

    New-ItemProperty `
        -LiteralPath $RegistryPath `
        -Name "Path" `
        -Value $NewPath `
        -PropertyType ExpandString `
        -Force | Out-Null

    $DummyName = "sysand-installer-" + [guid]::NewGuid().ToString()
    [Environment]::SetEnvironmentVariable($DummyName, "sysand-dummy", "User")
    [Environment]::SetEnvironmentVariable($DummyName, [NullString]::value, "User")
    return $true
}

function Show-NextSteps {
    param([string]$LiteralPath)

    if ($env:GITHUB_ACTIONS -eq "true") {
        return
    }

    Write-Host ""
    if ($NoModifyPath) {
        Write-Host "PATH was not modified. Add $LiteralPath to PATH to run sysand by name."
    } else {
        Write-Host "Open a new terminal to run sysand by name."
    }
    Write-Host ""
    Write-Host "For this already-open window, run the command for your shell:"
    Write-Host ""
    Write-Host "PowerShell:"
    Write-Host "  `$env:Path = `"$LiteralPath;`$env:Path`""
    Write-Host ""
    Write-Host "cmd.exe:"
    Write-Host "  set `"PATH=$LiteralPath;%PATH%`""
}

if ($Help) {
    Show-Usage
    exit 0
}

Initialize-Environment

if ($env:SYSAND_NO_MODIFY_PATH) {
    $NoModifyPath = $true
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    Fail "Version must not be empty."
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    Fail "Install directory must not be empty."
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
    $InstalledTemp = Join-Path $InstallDir "sysand-$([Guid]::NewGuid()).tmp"

    Write-Host "Installing sysand to $InstalledBinary"
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    try {
        Copy-Item -Path $ExtractedBinary -Destination $InstalledTemp -Force
        Move-Item -Path $InstalledTemp -Destination $InstalledBinary -Force
    } catch {
        if (Test-Path $InstalledTemp) {
            Remove-Item -Path $InstalledTemp -Force
        }
        throw
    }

    $InstalledVersion = & $InstalledBinary --version
    Write-Host "Installed $InstalledVersion"

    if (-not $NoModifyPath) {
        if (Add-CiPath $InstallDir) {
            Write-Host "Added $InstallDir to GITHUB_PATH."
        } elseif (Add-UserPath $InstallDir) {
            Write-Host "Added $InstallDir to your user PATH."
        }
    }

    Show-NextSteps $InstallDir
} finally {
    if (Test-Path $TempDir) {
        Remove-Item -Path $TempDir -Recurse -Force
    }
}
