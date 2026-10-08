#!/usr/bin/env python3
"""Exercise UDP receiving, Roto aggregation, reconnect and workspace restoration."""
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

root = Path(__file__).resolve().parent.parent
executable = root / "target/debug/tailer"
messages = [
    b"<34>Oct 11 22:14:15 bsd su: failed login",
    b"<189>123: Oct 11 22:14:15: %LINK-3-UPDOWN: Interface down",
    b"<132>Oct 11 22:14:15 juniper rpd[42]: RPD_BGP_NEIGHBOR_STATE_CHANGED",
    "<165>1 2026-10-11T22:14:15Z host app 42 ID47 - 日本語".encode(),
]
with tempfile.TemporaryDirectory(prefix="tailer-syslog-") as directory:
    env = {**os.environ, "HOME": directory, "APPDATA": directory,
           "LOCALAPPDATA": directory, "XDG_CONFIG_HOME": directory,
           "XDG_CACHE_HOME": directory, "QT_QPA_PLATFORM": "offscreen"}
    if sys.platform == "darwin" and os.environ.get("TAILER_QT_FRAMEWORK_PATH"):
        env["DYLD_FRAMEWORK_PATH"] = os.environ["TAILER_QT_FRAMEWORK_PATH"]
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("127.0.0.1", 0))
        address = probe.getsockname()
    endpoint = f"{address[0]}:{address[1]}"
    for _ in range(2):
        process = subprocess.Popen([str(executable), "--syslog-test", endpoint],
                                   env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and process.poll() is None:
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
                    try:
                        probe.bind(address)
                    except OSError:
                        break
                time.sleep(0.05)
            else:
                raise AssertionError("Syslog listener did not bind")
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
                for message in messages:
                    sender.sendto(message, address)
            stdout, stderr = process.communicate(timeout=20)
            print(stdout, end="")
            print(stderr, end="", file=sys.stderr)
            assert process.returncode == 0
            assert "TAILER_SYSLOG_OK" in stdout + stderr
            assert "TAILER_SYSLOG_FAILED" not in stdout + stderr
            assert "TypeError" not in stderr
            assert "ReferenceError" not in stderr
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind(address)