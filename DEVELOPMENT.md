# Developing Sysand

Requirements for contributing are specified in [CONTRIBUTING.md](CONTRIBUTING.md).

## Repository structure

The whole repository is a [Cargo
workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html) composed
of multiple crates (Rust packages) and variour other language libraries that
wrap the Rust core.

Directory structure:

- `core` (crate `sysand-core`) contains all the core logic, and can be used as
  a Rust library. It also contains (optional) coercion trait implementations for
  Python and WASM/JavaScript.
- `sysand` (crate `sysand`) wraps `sysand-core` into a user interface, currently
  a command line application.
- `bindings` contains wrappers for various programming languages:
  - `bindings/js` wraps `sysand-core` into a WASM/JavaScript library that can be
    used in Node, Deno, browsers, and so on.
  - `bindings/py` wraps `sysand-core` into a Python module.
  - `bindings/java` wraps `sysand-core` into a Java library.

  Note that the language libraries are currently in a very early state of
  development. Especially the JavaScript/WASM library is only a proof-of-concept
  that is not yet usable.

## Setup

To get the repository you need [Git](https://git-scm.com/) installed on your
system and then run:

```sh
git clone git@github.com:sensmetry/sysand.git
cd sysand
```

Development is done on Linux (including WSL), macOS or Windows. For Sysand core
and CLI development, you need to [install Rust](https://rust-lang.org/tools/install/)
specified in `rust-version` field of [Cargo.toml](Cargo.toml). It is also
recommended to use [`rust-analyzer`](https://github.com/rust-lang/rust-analyzer)
which is supported by many different editors.

### VS Code

If using [VS Code](https://code.visualstudio.com/) (or other compatible editor
like e.g. [VSCodium](https://vscodium.com/) or [Cursor](https://cursor.com/download))
we recommended the following extensions for developing in Rust:

- [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
  for Rust language support.
- [Error Lens](https://marketplace.visualstudio.com/items?itemName=usernamehw.errorlens)
  for improved highlighting of errors.
- [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)
  for debugging.
- [Even Better TOML](https://marketplace.visualstudio.com/items?itemName=tamasfe.even-better-toml)
  for `Cargo.toml` etc.

For rust-analyzer we can also recommend going into the settings and choosing
to activate the `Test Explorer` option to easily run tests in the editor.

Additionally you may be interested in using [Todo Tree](https://marketplace.visualstudio.com/items?itemName=Gruntfuggly.todo-tree)
for tracking `TODO` and `FIXME` comments. You may also want to go into the
REGEX setting and add `|todo!\(` to the end of the default regex to also catch
invocations of Rust's `todo!` macros.

All the extensions and more can be found in
[`.vscode/extensions.json`](.vscode/extensions.json) and these should show up
as recommended at the bottom of the extensions tab.

## Installing Sysand CLI

Sysand command line utility can be compiled from local repository and
installed as follows:

```console
$ cargo install --path=sysand
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
cargo test -p sysand-core -F filesystem,networking,js,python,alltests,kpar-bzip2,kpar-zstd,kpar-xz,kpar-ppmd
cargo test -p sysand -F alltests,kpar-bzip2,kpar-zstd,kpar-xz,kpar-ppmd
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

Format and lint all other code based on configuration in
`.pre-commit-config.yaml`, either with prek or pre-commit, available to install
via uv or pip.

```sh
prek run -a

# like this, you ensure this formatting is run before git commits are made
prek install
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

## Documentation

The "Sysand User Guide" is currently a work in progress. It is located in `docs/`.
Official version is hosted at [docs.sysand.org](https://docs.sysand.org/).
To preview it locally, make sure you have [`mdbook`](https://github.com/rust-lang/mdBook)
installed (`cargo install mdbook`), then either run

```sh
mdbook build docs/
```

and open `docs/book/index.html`, or run

```sh
mdbook serve docs/
```

and open [localhost:3000](http://localhost:3000/).

## Text formatting guide

Rules for formatting messages intended for the user and general
documentation.

### Log/error messages

This applies to messages printed to the user. It includes general
information, but also warnings and errors.

Rules to follow:

- error messages are both for users and developers,
  so should be comprehensible and useful to both
- if possible, provide user input (paths, file/dir names, arbitrary strings, IRIs, etc.)
  that caused the warning/error or is relevant to it; always quote user input using
  backticks (``)
- include as much information as reasonable, do not hide underlying errors
- messages generally start on a lowercase letter
- for multiline warnings, indent subsequent lines to align with the first, e.g.:

  ```rust
  const SP: char = ' ';
  log::warn!(
      "this is a very long warning\n\
      {SP:>8} message that spans\n\
      {SP:>8} multiple lines"
  );
  ```

- include explicit newlines in messages that are long or
  include potentially long interpolated values (see above example)

### Markdown

Rules for markdown (Rust doc comments, Markdown (`.md`) files):

- always include a language specifier in fenced code blocks,
  use `text` if no language is appropriate:

  ````md
  ```text

  ```
  ````

- for generic shell syntax highlighting using triple backticks,
  use `sh` language specifier:

  ````md
  ```sh

  ```
  ````

- for console session highlighting, such as

  ```console
  $ echo hello
  hello
  ```

  use `console` language specifier:

  ````md
  ```console

  ```
  ````
