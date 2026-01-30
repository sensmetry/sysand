# Installation

There are a few ways to download Sysand:

- From the Sysand website
- From PyPI
- From GitHub releases
- Compile from source

## Download latest release

Latest official Sysand CLI release can be downloaded from [beta.sysand.org][dl].
or from [latest GitHub release][gh_rel].

After downloading the appropriate file, installation depends on your platform:

- [Windows (both x86_64 and ARM64)](#windows)
- [macOS (both Intel and ARM64 (a.k.a. Apple Silicon))](#macos)
- [Linux (both x86_64 and ARM64)](#linux)

It is recommended to then [verify the installation](#verify-the-installation).

## Windows

The downloaded binary can either be installed manually or by running
a few PowerShell commands.

### Manual installation

1. Move the downloaded `.exe` file to `%LOCALAPPDATA%\Programs\Sysand\sysand.exe`
2. Add to `PATH` via Environment Variables:

    1. Open "Environment Variables" (search in Start menu)
    2. Under "User variables", select "Path" and click "Edit"
    3. Click "New" and add `%LOCALAPPDATA%\Programs\Sysand`
    4. Click "Ok" to save

### PowerShell installation

1. Open PowerShell
2. Run these commands:

```powershell
# For x86_64 systems

# Create directory and move to it
mkdir "$env:LOCALAPPDATA\Programs\Sysand" -Force
mv sysand-windows-x86_64.exe "$env:LOCALAPPDATA\Programs\Sysand\sysand.exe"

# For ARM64 systems

# Create directory and move to it
mkdir "$env:LOCALAPPDATA\Programs\Sysand" -Force
mv sysand-windows-arm64.exe "$env:LOCALAPPDATA\Programs\Sysand\sysand.exe"
```

3. Then run:

```powershell
# Add to PATH
$currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
$newPath = "$env:LOCALAPPDATA\Programs\Sysand"
if ($currentPath -notlike "*$newPath*") {
   [Environment]::SetEnvironmentVariable("Path", "$currentPath;$newPath", "User")
}
```

> [!important]
> Restart your terminal after installation for `PATH` changes to take effect.


## macOS

### System installation (requires `sudo`)

1. Open Terminal
2. Make the binary executable and move to a folder in `PATH` by running the
    following commands:

```sh
# For Intel Macs
chmod +x ~/Downloads/sysand-macos-x86_64
sudo mv ~/Downloads/sysand-macos-x86_64 /usr/local/bin/sysand

# For Apple Silicon Macs
chmod +x ~/Downloads/sysand-macos-arm64
sudo mv ~/Downloads/sysand-macos-arm64 /usr/local/bin/sysand
```

### Alternative: user installation (no `sudo` required)

It is also possible to install without `sudo` rights, but that requires
additional steps.

1. Run in Terminal:

```sh
# Create local bin directory if it doesn't exist
mkdir -p ~/.local/bin

# Add local bin directory to PATH in your shell profile
# (default ~/.zshrc, but could be ~/.bashrc etc.)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

2. Move the executable as shown above for the system installation,
    but instead of `/usr/local/bin/sysand` move to `~/.local/bin/sysand`.


## Linux

### System installation (requires `sudo`)

1. Open a terminal
2. Make the binary executable and move to a folder in `PATH` by running the
    following commands:

```sh
# For x86_64 systems
wget https://github.com/sensmetry/sysand/releases/latest/download/sysand-linux-x86_64
chmod +x sysand-linux-x86_64
sudo mv sysand-linux-x86_64 /usr/local/bin/sysand

# For ARM64 systems
wget https://github.com/sensmetry/sysand/releases/latest/download/sysand-linux-arm64
chmod +x sysand-linux-arm64
sudo mv sysand-linux-arm64 /usr/local/bin/sysand
```

### Alternative: user installation (no `sudo` required)

It is also possible to install without `sudo` rights, but that requires
additional steps.

1. Run in a terminal:

```sh
# Create local bin directory if it doesn't exist
mkdir -p ~/.local/bin

# Add local bin directory to PATH in your shell profile (~/.bashrc, ~/.zshrc, etc.)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

2. Move the executable as shown above for the system installation,
    but instead of `/usr/local/bin/sysand` move to `~/.local/bin/sysand`.


## Verify the installation

Run in a terminal:

```sh
sysand --version
```

You should see an output similar to: `sysand X.Y.Z` 


## PyPI

Sysand is [published on PyPI](https://pypi.org/project/sysand/) and includes both
the CLI and Sysand Python library.

It can be installed using [pip](https://pip.pypa.io/en/stable/). Run the following
in the terminal:

```sh
pip install sysand
```

Alternatively, [uv](https://docs.astral.sh/uv/) can be used for this:

```sh
uv pip install sysand
```

## Download development version

Latest development version of Sysand can be downloaded from
[GitHub releases][gh_rel_all] by choosing the latest release by date
(which is usually labelled starting with "Nightly Release").

## Compiling from source code

Sysand is written in Rust programming language and so can be installed
using [cargo](https://doc.rust-lang.org/cargo/). Cargo is part of
[Rust language tooling](https://www.rust-lang.org/tools/install).
If you have it, run the following command in the terminal:

```sh
cargo install sysand --git=https://github.com/sensmetry/sysand.git
```

[dl]: https://beta.sysand.org/download.html
[gh_rel]: https://github.com/sensmetry/sysand/releases/latest
[gh_rel_all]: https://github.com/sensmetry/sysand/releases/
