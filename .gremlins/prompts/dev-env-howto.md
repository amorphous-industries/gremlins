## Development environment

A Python virtual environment is available at `.venv` in the working directory.
To use it in any shell command, prefix with a source:

```bash
source .venv/bin/activate && <your command>
```

Or use the venv Python directly:

```bash
.venv/bin/python -m pytest tests/test_foo.py
```

The venv has the project installed in editable mode with all dev dependencies
(ruff, pyright, pytest, maturin). Use `make check` and `make test` to verify
your work.