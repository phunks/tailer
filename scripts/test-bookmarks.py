#!/usr/bin/env python3
"""Check bookmark persistence across real application restarts in an isolated HOME.

Build Tailer first and configure Qt's library paths for your environment.
An optional first argument selects a different executable.
On macOS, TAILER_QT_FRAMEWORK_PATH can supply Qt's lib directory when system
Python strips DYLD_FRAMEWORK_PATH from its inherited environment.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


root = Path(__file__).resolve().parent.parent
executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/tailer"
with tempfile.TemporaryDirectory(prefix="tailer-bookmarks-") as directory:
    env = {**os.environ, "HOME": directory, "QT_QPA_PLATFORM": "offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    # APPDATA/XDG_CONFIG_HOME are also isolated when running outside macOS.
    env["APPDATA"] = directory
    env["LOCALAPPDATA"] = directory
    env["XDG_CONFIG_HOME"] = directory
    env["XDG_CACHE_HOME"] = directory
    config = Path(directory)
    if sys.platform == "darwin":
        config /= "Library/Application Support"
    state_path = config / "Tailer/workspace.json"
    for mode, expected_count in [("save", 1), ("open", 0)]:
        result = subprocess.run(
            [str(executable), "--bookmark-test", mode],
            env=env, capture_output=True, text=True, timeout=30,
        )
        print(result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        result.check_returncode()
        assert "ReferenceError" not in result.stdout + result.stderr
        marker = "TAILER_BOOKMARK_SAVE_OK" if mode == "save" else "TAILER_BOOKMARK_OPEN_DELETE_OK"
        assert marker in result.stdout + result.stderr
        state = json.loads(state_path.read_text())
        assert len(state["bookmarks"]) == expected_count
        assert state["tabs"] == []
        if mode == "save":
            assert state["bookmarks"][0]["bookmarkTitle"] == "本番ログ / My bookmark"
            assert state["bookmarks"][0]["logTitle"] == "Bookmark test"
    for mode, marker in [("groups-save", "TAILER_GROUP_SAVE_OK"), ("groups-open", "TAILER_GROUP_OPEN_DELETE_OK")]:
        result = subprocess.run([str(executable), "--bookmark-test", mode],
                                env=env, capture_output=True, text=True, timeout=30)
        print(result.stdout, end="")
        print(result.stderr, end="", file=sys.stderr)
        result.check_returncode()
        assert "ReferenceError" not in result.stdout + result.stderr
        assert marker in result.stdout + result.stderr
        state = json.loads(state_path.read_text())
        assert state["tabs"] == []
        assert len(state["bookmarks"]) == 2
        if mode == "groups-save":
            assert len(state["bookmarkGroups"]) == 2
            assert state["bookmarkGroups"][0]["expanded"] is False
        else:
            assert state["bookmarkGroups"] == []
            assert all(bookmark["groupId"] == "" for bookmark in state["bookmarks"])