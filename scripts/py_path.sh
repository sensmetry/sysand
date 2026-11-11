# SPDX-License-Identifier: MIT OR Apache-2.0
# SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

# shellcheck shell=bash

# Workaround to make pyo3 embedded Python detect venv python libs.
# See https://github.com/PyO3/pyo3/issues/1741
OS="$(uname -s)"
if [[ "$OS" == "Linux" ]]; then
    PYO3_PYTHON="$(uv run python -c "import sys; print(sys.executable)")"
    LD_LIBRARY_PATH="$(uv run python -c "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))")"
    PYTHONHOME="$(uv run python -c "import sys; print(sys.base_prefix)")"
    export PYO3_PYTHON LD_LIBRARY_PATH PYTHONHOME
elif [[ "$OS" == "Darwin" && -n "${VIRTUAL_ENV-}" ]]; then
    PYO3_PYTHON="$(uv run python -c "import sys; print(sys.executable)")"
    PYTHONHOME="$(uv run python -c "import sys; print(sys.base_prefix)")"
    export PYO3_PYTHON PYTHONHOME
fi
