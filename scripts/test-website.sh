#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

bash -n "$ROOT/scripts/generate-brand-assets.sh"
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT/scripts/capture-website.py" --help >/dev/null
PYTHONDONTWRITEBYTECODE=1 python3 - "$ROOT/scripts/capture-website.py" <<'PY'
import importlib.util
import os
from pathlib import Path
import sys
import tempfile

script = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("capture_website", script)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)

executable = Path(sys.executable)
assert module.resolve_chrome(executable) == executable

saved_chrome = os.environ.get("BLACKPEPPER_CHROME")
saved_path = os.environ.get("PATH", "")
try:
    os.environ["BLACKPEPPER_CHROME"] = str(executable)
    assert module.resolve_chrome(None) == executable

    os.environ.pop("BLACKPEPPER_CHROME")
    with tempfile.TemporaryDirectory(prefix="blackpepper-chrome-path-") as root:
        discovered = Path(root) / "chromium"
        discovered.symlink_to(executable)
        os.environ["PATH"] = root
        assert module.resolve_chrome(None) == discovered

        discovered.unlink()
        try:
            module.resolve_chrome(None)
        except SystemExit as error:
            assert "--chrome /absolute/path/to/chrome" in str(error)
        else:
            raise AssertionError("missing Chrome unexpectedly resolved")
finally:
    os.environ["PATH"] = saved_path
    if saved_chrome is None:
        os.environ.pop("BLACKPEPPER_CHROME", None)
    else:
        os.environ["BLACKPEPPER_CHROME"] = saved_chrome
PY
python3 "$ROOT/scripts/validate-website.py"
