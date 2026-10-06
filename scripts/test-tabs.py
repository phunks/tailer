#!/usr/bin/env python3
"""Check live tab moves preserve sessions and context-menu close operations.

Build first. On macOS set TAILER_QT_FRAMEWORK_PATH to Qt's lib directory.
Pass an optional executable path as the first argument.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


root = Path(__file__).resolve().parent.parent
executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/tailer"
with tempfile.TemporaryDirectory(prefix="tailer-tabs-") as directory:
    env = {**os.environ, "HOME": directory, "APPDATA": directory,
           "LOCALAPPDATA": directory, "XDG_CONFIG_HOME": directory,
           "XDG_CACHE_HOME": directory, "QT_QPA_PLATFORM": "offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    result = subprocess.run([str(executable), "--tab-ui-test"], env=env,
                            capture_output=True, text=True, timeout=30)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    assert "TAILER_TAB_UI_OK" in result.stdout + result.stderr
    config = Path(directory)
    if sys.platform == "darwin":
        config /= "Library/Application Support"
    assert json.loads((config / "Tailer/workspace.json").read_text())["tabs"] == []