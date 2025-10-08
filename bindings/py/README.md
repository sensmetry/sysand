# Python bindings

## Building and running tests

Requirements:

- Rust 1.85 or later
- uv

First, set up a Python venv:

```bash
uv venv
source .venv/bin/activate
uv pip install -r requirements-dev.txt
```

Rust/"native" tests use PyO3, which does not work well within a Python venv.
It is therefore recommended to use the supplied script to run all ("native" and pytest) tests:

```bash
./scripts/run_tests.sh
```

Alternatively, to build and run Python tests:

```bash
maturin develop
pytest
```

Rust/"native" tests must be run without the `extension-module` feature:

```bash
cargo test --no-default-features
```

If this is run inside a venv and does not work, look in `scripts/run_tests.sh` for fixes.


## Formatting and linting

Format Rust and Python code and run linters for both:

```bash
./scripts/run_chores.sh
```
