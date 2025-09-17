# Python bindings

To build and run Python tests (using uv):

```bash
uv venv
source .venv/bin/activate
uv pip install -r requirements-dev.txt
maturin develop
pytest
```

There are also "native" tests, which must be run without
the `extension-module` feature:

```bash
cargo test --no-default-features
```

To run both "native" and pytest tests, use the supplied script

```bash
source .venv/bin/activate
./run_tests.sh
```
