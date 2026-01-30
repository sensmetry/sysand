# Installation

There are a few ways to download Sysand releases.

## Download latest release

Latest official Sysand CLI release can be downloaded from [beta.sysand.org][dl].
It does not require any installation.

Alternatively, Sysand can be downloaded from [GitHub releases][gh_rel].

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
using [cargo](https://doc.rust-lang.org/cargo/). Cargo is installed
as part of [Rust language tooling](https://www.rust-lang.org/tools/install).
Once you have it, run the following command in the terminal:

```sh
cargo install sysand --git=https://github.com/sensmetry/sysand.git
```

[dl]: https://beta.sysand.org/download.html
[gh_rel]: https://github.com/sensmetry/sysand/releases/latest
[gh_rel_all]: https://github.com/sensmetry/sysand/releases/
