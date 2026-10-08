#!/usr/bin/env python3
"""Exercise production Roto analysis controls, regrouping, errors and live input."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

root = Path(__file__).resolve().parent.parent
executable = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/debug/tailer"
with tempfile.TemporaryDirectory(prefix="tailer-analysis-") as directory:
    log = Path(directory) / "access.log"
    prefix = '2026-10-07T11:37:54.236480684Z INFO:     123.123.123.123:57260 - '
    log.write_text(prefix + '"HEAD /ui/policies/?page=1 HTTP/1.1" 200 OK\n'
                   + prefix + '"GET /ui/policies/?page=2 HTTP/1.1" 500 Error\n'
                   + 'not an access log\n'
                   + '{"event":"checkpoint","timing":{"total":5.442}}\n'
                   + 'CSV:,checkpoint,2.5\n'
                   + 'WS:   checkpoint\t3.5\n')
    env = {**os.environ, "HOME": directory, "APPDATA": directory,
           "LOCALAPPDATA": directory, "XDG_CONFIG_HOME": directory,
           "XDG_CACHE_HOME": directory, "QT_QPA_PLATFORM": "offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    def append():
        with log.open("a") as stream:
            stream.write(prefix + '"GET /ui/policies/?page=3 HTTP/1.1" 200 OK\n')
    timer = threading.Timer(5, append)
    timer.start()
    try:
        result = subprocess.run([str(executable), "--analysis-test", str(log)],
                                env=env, capture_output=True, text=True, timeout=35)
    finally:
        timer.cancel()
        timer.join()
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    assert "TAILER_ANALYSIS_OK" in result.stdout + result.stderr
    restored = subprocess.run([str(executable), "--analysis-test", str(log),
                               "--analysis-preset-restore"], env=env,
                              capture_output=True, text=True, timeout=20)
    print(restored.stdout, end="")
    print(restored.stderr, end="", file=sys.stderr)
    restored.check_returncode()
    assert "TAILER_ANALYSIS_PRESET_RESTORE_OK" in restored.stdout + restored.stderr