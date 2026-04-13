# Developing Sysand

Requirements for contributing are specified in
[CONTRIBUTING.md](CONTRIBUTING.md), and overview of the project's architecture
is maintained in [ARCHITECTURE.md](ARCHITECTURE.md).

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
their respective folders under a DEVELOPMENT.md or README.md:

- [Java](bindings/java/README.md)
- [Python](bindings/py/DEVELOPMENT.md)
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

Format and lint all code based on configuration in `.pre-commit-config.yaml`
with `prek` ([installation options]).

```sh
prek run -a

# like this, you ensure this formatting is run before git commits are made
prek install
```

[installation options]: https://github.com/j178/prek?tab=readme-ov-file#installation

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

## Rust code structure

When adding or refactoring Rust code, organize modules so the public API appears
before private implementation details. If a module's public API is mainly
functions, for example command modules, put those entry-point functions first
and place supporting public types after them.

Recommended order in a file:

- declarations at the top (`use`, `mod`, and test module declaration)
- public API next, ordered from primary entry points to supporting items
- private implementation details (`const`, private helper functions/types)

For most command modules, prefer this public API order:

- primary entry-point functions first (`pub fn`), followed by
- supporting public types (`pub enum`, `pub struct`, type aliases)

For type-centric modules (where the type is the main abstraction), it is fine
to put the core public type(s) before methods/functions.

If unsure, default to function-first in command modules.

For unit tests, prefer dedicated test files for non-trivial modules instead of
large inline `#[cfg(test)]` blocks.

- use a sibling test file named `<module>_tests.rs`
- declare tests from the main module using:

  ```rust
  #[cfg(test)]
  #[path = "./<module>_tests.rs"]
  mod tests;
  ```

- keep the module name as `tests` for consistency across files
- keep test-only helpers in the test file unless they are shared broadly

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
