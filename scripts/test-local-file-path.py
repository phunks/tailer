#!/usr/bin/env python3
"""Exercise the production file URL conversion for Windows and POSIX paths."""
import os
from pathlib import Path
import subprocess
import tempfile

source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
start = source.index("    function localFilePath(")
end = source.index("\n    function addLog(", start)
assert "const path = root.localFilePath(url);" in source
fixture = """
import QtQuick
import QtTest
Item {
    id: root
""" + source[start:end] + """
    TestCase {
        name: "LocalFilePath"
        function test_conversion_data() {
            return [
                {tag: "windows-drive", url: "file:///D:/logs/app.log", platform: "windows", path: "D:/logs/app.log"},
                {tag: "windows-lowercase-drive", url: "file:///c:/logs/app.log", platform: "windows", path: "c:/logs/app.log"},
                {tag: "windows-localhost", url: "file://localhost/D:/logs/app.log", platform: "windows", path: "D:/logs/app.log"},
                {tag: "windows-unicode", url: "file:///D:/my%20logs/%E6%97%A5%E6%9C%AC%E8%AA%9E.log", platform: "windows", path: "D:/my logs/日本語.log"},
                {tag: "windows-unc", url: "file://server/share/my%20log.log", platform: "windows", path: "//server/share/my log.log"},
                {tag: "windows-unc-four-slashes", url: "file:////server/share/app.log", platform: "windows", path: "//server/share/app.log"},
                {tag: "localhost-prefix-is-host", url: "file://localhost-server/share/app.log", platform: "windows", path: "//localhost-server/share/app.log"},
                {tag: "macos", url: "file:///Users/test/my%20log.log", platform: "osx", path: "/Users/test/my log.log"},
                {tag: "linux", url: "file:///var/log/app.log", platform: "linux", path: "/var/log/app.log"},
                {tag: "posix-drive-like-name", url: "file:///D:/logs/app.log", platform: "linux", path: "/D:/logs/app.log"},
                {tag: "encoded-percent", url: "file:///D:/logs/100%25%23%3F.log", platform: "windows", path: "D:/logs/100%#?.log"}
            ];
        }
        function test_conversion(data) {
            compare(root.localFilePath(data.url, data.platform), data.path);
        }
        function test_native_platform() {
            compare(root.localFilePath("file:///D:/logs/app.log"),
                    Qt.platform.os === "windows" ? "D:/logs/app.log" : "/D:/logs/app.log");
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-path-test-") as directory:
    path = Path(directory) / "tst_LocalFilePath.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)