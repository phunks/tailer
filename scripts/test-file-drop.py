#!/usr/bin/env python3
"""Exercise production file-drop routing and URL conversion with Qt's JS engine."""
import os
from pathlib import Path
import subprocess
import tempfile

source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
path_start = source.index("    function localFilePath(")
path_end = source.index("\n    function addLog(", path_start)
drop_start = source.index("    function hasLocalFileUrls(")
drop_end = source.index("\n    Shortcut {", drop_start)
fixture = """
import QtQuick
import QtTest
Item {
    id: root
    width: 800; height: 600
    property var contentItem: root
    property var opened: []
    function addLog(path) { opened.push(path); }
""" + source[path_start:path_end] + source[drop_start:drop_end] + """
    TestCase {
        name: "FileDrop"
        function init() { root.opened = []; }
        function event(urls, hasUrls = true) {
            return {urls: urls, hasUrls: hasUrls, accepted: true, completed: false, action: Qt.IgnoreAction,
                    accept: function(action) { this.action = action; this.completed = true; }};
        }
        function test_multiple_files() {
            const drop = event(["file:///tmp/first.log", "file:///tmp/my%20log.log"]);
            root.dropLocalFiles(drop);
            compare(root.opened.length, 2);
            compare(root.opened[0], "/tmp/first.log");
            compare(root.opened[1], "/tmp/my log.log");
            verify(drop.completed);
            compare(drop.action, Qt.CopyAction);
        }
        function test_windows_and_unc() {
            const drop = event(["file:///D:/logs/%E6%97%A5%E6%9C%AC%E8%AA%9E.log", "file://server/share/app.log"]);
            root.dropLocalFiles(drop);
            compare(root.opened[0], Qt.platform.os === "windows" ? "D:/logs/日本語.log" : "/D:/logs/日本語.log");
            compare(root.opened[1], "//server/share/app.log");
            verify(drop.completed);
        }
        function test_reject_data() {
            return [
                {tag: "web", urls: ["https://example.com/app.log"], hasUrls: true},
                {tag: "empty", urls: [], hasUrls: true},
                {tag: "text", urls: [], hasUrls: false}
            ];
        }
        function test_reject(data) {
            const drop = event(data.urls, data.hasUrls);
            root.dropLocalFiles(drop);
            compare(root.opened.length, 0);
            verify(!drop.accepted);
            verify(!drop.completed);
        }
        function test_mixed_urls() {
            const drop = event(["https://example.com/app.log", "file:///tmp/app.log"]);
            root.dropLocalFiles(drop);
            compare(root.opened.length, 1);
            compare(root.opened[0], "/tmp/app.log");
            verify(drop.completed);
        }
        function test_drag_filter() {
            verify(root.hasLocalFileUrls(["file:///tmp/app.log"]));
            verify(!root.hasLocalFileUrls(["https://example.com/app.log"]));
            verify(!root.hasLocalFileUrls([]));
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-drop-test-") as directory:
    path = Path(directory) / "tst_FileDrop.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)