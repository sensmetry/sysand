# Developing Sysand

Requirements for contributing are specified in [CONTRIBUTING.md](CONTRIBUTING.md).

## Repository structure

The whole repository is a [Cargo
workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html) composed
of multiple crates (Rust packages).

Directory structure:

- `core` contains all the core logic, and can be used as a Rust library. It also
  contains (optional) coercion trait implementations for Python and
  WASM/JavaScript.
- `sysand` wraps `core` into a user interface, currently a command line
  application.
- `bindings` wraps `core` for various programming languages:

  - `bindings/js` wraps `core` into a WASM/JavaScript library that can be used
    in Node, Deno, browsers, and so on.
  - `bindings/py` wraps `core` into a Python module.
  - `bindings/java` wraps `core` into a Java library.

  Note that the language libraries are currently in a very early state of
  development. Especially the JavaScript/WASM library is only a proof-of-concept
  that is not yet usable.

## Setup

Development is done on Linux (including WSL) or macOS. For Sysand core and CLI
development, you need to [install Rust](https://rust-lang.org/tools/install/)
1.85 or later and [uv](https://docs.astral.sh/uv/). It is also recommended
to use [`rust-analyzer`](https://github.com/rust-lang/rust-analyzer).
It has an [extension for VS
Code](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyze
r) and many other code editors can use it via
[LSP](https://microsoft.github.io/language-server-protocol/).
Other useful VS Code extensions can be found in
[`.vscode/extensions.json`](.vscode/extensions.json).

Get the repository:
```sh
git clone git@github.com:sensmetry/sysand.git
cd sysand
```

## Installing Sysand CLI

Sysand command line utility can be compiled from local repository and
installed as follows:
```sh
cargo install --path=sysand
[...]
Installed package `sysand vX.Y.Z (/...)` (executable `sysand`)
```
It is then available as `sysand` from the command line.

## Language bindings

Instructions for developing language bindings are specified in
their respective READMEs:

- [Java](bindings/java/README.md)
- [Python](bindings/py/README.md)
- [JavaScript (WASM)](bindings/js/README.md)

## Building

Build the Sysand CLI:
```sh
cargo build -p sysand # unoptimized
# or
cargo build -p sysand --release # optimized
```

Build binaries of all Rust crates in the workspace:
```sh
cargo build # unoptimized
# or
cargo build --release # optimized
```

## Running tests

Run tests for main Rust crates. This excludes language bindings, because they
have their own test suites:
```sh
cargo test -p sysand-core -F filesystem,js,python,alltests
cargo test -p sysand -F alltests
```

Run tests for all crates and language bindings (requires bindings dependencies):
```sh
./scripts/run_tests.sh
```

## Formatting and linting

Format Rust code in core crates:
```sh
cargo fmt -p sysand-core -p sysand
```

Format and lint all Rust and bindings code (requires bindings dependencies):
```sh
./scripts/run_chores.sh
```

## Commits and pull requests

Committing your changes:
```sh
git commit -sm "your commit message"
```
The `-s` flag signs the commit, see [CONTRIBUTING.md](CONTRIBUTING.md).

Pull requests must pass CI and be reviewed by a maintainer to be
merged. The project uses GitHub Actions CI, its configuration is in
[`.github/workflows` directory](.github/workflows). The CI runs the same tests
as [`./scripts/run_tests.sh`](scripts/run_tests.sh). Therefore it is recommended
to make sure that all tests pass locally before submitting a pull request.
