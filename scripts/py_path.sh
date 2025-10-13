# Workaround to make pyo3 embedded Python detect venv python libs.
# See https://github.com/PyO3/pyo3/issues/1741
OS="$(uname -s)"
if [[ "$OS" == "Linux" ]]; then
    export LD_LIBRARY_PATH="$(uv run python -c "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))")"
elif [[ "$OS" == "Darwin" && -n "${VIRTUAL_ENV-}" ]]; then
    echo a
    export PYTHONHOME="$(uv run python -c "import sys; print(sys.base_prefix)")"
fi
