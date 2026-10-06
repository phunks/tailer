#!/usr/bin/env python3
"""Exercise production appearance controls and restart persistence.

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
with tempfile.TemporaryDirectory(prefix="tailer-appearance-") as directory:
    env = {**os.environ, "HOME": directory, "APPDATA": directory,
           "LOCALAPPDATA": directory, "XDG_CONFIG_HOME": directory,
           "XDG_CACHE_HOME": directory, "QT_QPA_PLATFORM": "offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    config = Path(directory)
    if sys.platform == "darwin":
        config /= "Library/Application Support"
    state_path = config / "Tailer/workspace.json"

    def run(mode):
        result = subprocess.run([str(executable), "--theme-test", mode],
                                env=env, capture_output=True, text=True, timeout=30)
        print(result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        result.check_returncode()
        marker = "TAILER_THEME_CYCLE_OK" if mode == "cycle" else "TAILER_THEME_RESTORE_OK"
        assert marker in result.stdout + result.stderr

    run("cycle")
    state = json.loads(state_path.read_text())
    assert state["appearance"] == "auto"
    for mode in ["dark", "light", "auto"]:
        state["appearance"] = mode
        state_path.write_text(json.dumps(state))
        run(mode)
        assert json.loads(state_path.read_text())["appearance"] == mode
    # Configuration created before appearance was introduced defaults to Auto.
    del state["appearance"]
    state_path.write_text(json.dumps(state))
    run("auto")