# Installation

There are a few ways to download Sysand:

- From PyPI
- From this page
- From GitHub releases
- Compile from source

## PyPI

Sysand is [published to PyPI](https://pypi.org/project/sysand/) and includes
both the CLI and Sysand Python library.

We recommend installing from PyPI into an isolated (virtual) environment. This
can be done with [pipx](https://pipx.pypa.io/stable/installation/):

```sh
pipx install sysand
```

Or with [uv](https://docs.astral.sh/uv/):

```sh
uv tool install sysand
```

> [!note]
> `pipx`/`uv tool` will make only the CLI part of the package accessible

Both Sysand CLI and Python library can be installed with
[pip](https://pip.pypa.io/en/stable/installation/):

```sh
pip install sysand
```

> [!tip]
> `pip` is sometimes called `pip3`, so if `pip` command is not available, try `pip3`

Or with `uv` (run inside a virtual environment):

```sh
uv pip install sysand
```

## Download latest release

Latest official Sysand CLI release can be downloaded below
or from [latest GitHub release][gh_rel].

<!-- markdownlint-disable MD033 MD013 -->
<table>
  <tr>
    <th></th>
    <th><i class="fab fa-windows"></i> Windows</th>
    <th><i class="fab fa-apple"></i> macOS</th>
    <th><i class="fab fa-linux"></i> Linux</th>
  </tr>
  <tr>
    <td><strong>x86_x64</strong></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-windows-x86_64.zip"><button><i class="fas fa-download"></i> Download</button></a></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-macos-x86_64.tar.xz"><button><i class="fas fa-download"></i> Download</button></a></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-linux-x86_64.tar.xz"><button><i class="fas fa-download"></i> Download</button></a></td>
  </tr>
  <tr>
    <td><strong>ARM64</strong></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-windows-arm64.zip"><button><i class="fas fa-download"></i> Download</button></a></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-macos-arm64.tar.xz"><button><i class="fas fa-download"></i> Download</button></a></td>
    <td><a href="https://github.com/sensmetry/sysand/releases/latest/download/sysand-linux-arm64.tar.xz"><button><i class="fas fa-download"></i> Download</button></a></td>
  </tr>
</table>
<!-- markdownlint-enable MD033 MD013 -->

The download is an archive (`.zip` on Windows, `.tar.xz` on macOS and
Linux) containing a single `sysand` executable (`sysand.exe` on
Windows). After downloading, extract the archive and follow the steps
for your platform:

- [Windows (both x86_64 and ARM64)](#windows)
- [macOS (both Intel and ARM64 (a.k.a. Apple Silicon))](#macos)
- [Linux (both x86_64 and ARM64)](#linux)

It is recommended to then [verify the installation](#verify-the-installation).

### Windows

The downloaded `.zip` archive contains `sysand.exe`. It can either be
installed manually or by running a few PowerShell commands.

#### Manual installation

1. Right-click the downloaded `.zip` file and choose "Extract All..."
2. Move the extracted `sysand.exe` to `%LOCALAPPDATA%\Programs\Sysand\sysand.exe`
3. Add to `PATH` via Environment Variables:
   1. Open "Environment Variables" (search in Start menu)
   2. Under "User variables", select "Path" and click "Edit"
   3. Click "New" and add `%LOCALAPPDATA%\Programs\Sysand`
   4. Click "Ok" to save

#### PowerShell installation

1. Open PowerShell
2. Run these commands:

```powershell
# For x86_64 systems

# Create directory and extract the archive into it
mkdir "$env:LOCALAPPDATA\Programs\Sysand" -Force
Expand-Archive -Path sysand-windows-x86_64.zip `
  -DestinationPath "$env:LOCALAPPDATA\Programs\Sysand" -Force

# For ARM64 systems

# Create directory and extract the archive into it
mkdir "$env:LOCALAPPDATA\Programs\Sysand" -Force
Expand-Archive -Path sysand-windows-arm64.zip `
  -DestinationPath "$env:LOCALAPPDATA\Programs\Sysand" -Force
```

3. Add folder to `PATH`:

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

### macOS

#### System installation (requires `sudo`)

1. Open Terminal
2. Extract the archive and move the `sysand` binary to a folder in
   `PATH` by running the following commands:

```sh
# For Intel Macs
tar -xJf ~/Downloads/sysand-macos-x86_64.tar.xz -C ~/Downloads
sudo mv ~/Downloads/sysand /usr/local/bin/sysand

# For Apple Silicon Macs
tar -xJf ~/Downloads/sysand-macos-arm64.tar.xz -C ~/Downloads
sudo mv ~/Downloads/sysand /usr/local/bin/sysand
```

#### Alternative: user installation (no `sudo` required)

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

### Linux

#### System installation (requires `sudo`)

1. Open a terminal
2. Extract the archive and move the `sysand` binary to a folder in
   `PATH` by running the following commands:

```sh
# For x86_64 systems
tar -xJf sysand-linux-x86_64.tar.xz
sudo mv sysand /usr/local/bin/sysand

# For ARM64 systems
tar -xJf sysand-linux-arm64.tar.xz
sudo mv sysand /usr/local/bin/sysand
```

#### Alternative: user installation (no `sudo` required)

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

### Verify the installation

Run in a terminal:

```sh
sysand --version
```

You should see an output similar to: `sysand X.Y.Z`

## Download development version

Latest development version of Sysand can be downloaded from the
[`nightly` GitHub release][gh_rel_nightly].

> [!warning]
> Nightly assets are raw signing inputs for release automation. They are not
> packaged like the official release downloads above.

## Compiling from source code

Sysand is written in Rust programming language and so can be installed
using [cargo](https://doc.rust-lang.org/cargo/). Cargo is part of
[Rust language tooling](https://www.rust-lang.org/tools/install).
If you have it, run the following command in the terminal:

```sh
cargo install sysand --git=https://github.com/sensmetry/sysand.git
```

[gh_rel]: https://github.com/sensmetry/sysand/releases/latest
[gh_rel_nightly]: https://github.com/sensmetry/sysand/releases/tag/nightly
