# Python bindings

## Building and running tests

Requirements:

- Rust version given in `rust-version` in [Cargo.toml](../../Cargo.toml) or later
- uv

First, set up a Python venv:

```bash
uv venv
source .venv/bin/activate # or e.g. .venv/bin/activate.fish depending on your shell
```

Rust/"native" tests use PyO3, which does not work well within a Python venv.
It is therefore recommended to use the supplied script to run all ("native"
and pytest) tests:

```sh
./scripts/run_tests.sh
```

Alternatively, to build and run Python tests:

```sh
uv run maturin develop
uv run pytest
```

Rust/"native" tests must be run without the `extension-module` feature:

```sh
cargo test --no-default-features
```

If this is run inside a venv and does not work, look in `scripts/run_tests.sh` for fixes.

## Formatting and linting

Format Rust and Python code and run linters for both:

```sh
./scripts/run_chores.sh
```

## Changing Python version

Python version used by default for venvs is specified in `.python-version`.
If you change the version there, you should run

```sh
cargo clean -p pyo3-build-config
```

to ensure that no references to previously used Python version remain in build cache.
