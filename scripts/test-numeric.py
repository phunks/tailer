#!/usr/bin/env python3
"""Exercise numeric filter/trap controls, live split output, and bookmark saving.

Build first. On macOS set TAILER_QT_FRAMEWORK_PATH to Qt's lib directory.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parent.parent
executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/tailer"
with tempfile.TemporaryDirectory(prefix="tailer-numeric-") as directory:
    env = {**os.environ, "HOME":directory, "APPDATA":directory, "LOCALAPPDATA":directory,
           "XDG_CONFIG_HOME":directory, "XDG_CACHE_HOME":directory, "QT_QPA_PLATFORM":"offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    for phase in ["save", "restore"]:
        result = subprocess.run([str(executable), "--numeric-test"], env=env, capture_output=True, text=True, timeout=30)
        print(phase, result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        result.check_returncode()
        assert "TAILER_NUMERIC_OK" in result.stdout + result.stderr