#!/usr/bin/env python3
"""Exercise production per-tab encoding controls and new-log defaults."""

import os
from pathlib import Path
import subprocess
import tempfile

source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
start = source.index("    function addLog(")
end = source.index("\n    function closeTab(", start)
add_log = source[start:end]
start = source.index("    function logConfiguration(")
end = source.index("\n    function ", start + 1)
configuration = source[start:end]
start = source.index("                            RowLayout {\n                                id: encodingControls")
end = source.index("                            CheckBox {", start)
controls = source[start:end]
options = next(line for line in source.splitlines() if "property var encodingOptions:" in line)

fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    width: 800; height: 200
    property int saves: 0
    function scheduleSave() { saves++; }
    QtObject { id: settings; property int initialLines: 50; property int capacityMiB: 512 }
    QtObject { id: tabs; property int currentIndex: -1 }
    ListModel { id: logs }
    QtObject {
        id: page
        property int index: 0
        property string source: logs.count > index ? logs.get(index).source : "file"
        property string encoding: logs.count > index ? logs.get(index).encoding : "UTF-8"
    }
    QtObject {
        id: backend
        property var changes: []
        function set_encoding(value) { changes = changes.concat([value]); }
    }
""" + options + "\n" + add_log + configuration + controls + """
    TestCase {
        name: "EncodingControls"
        when: windowShown
        function init() {
            logs.clear();
            root.addLog("/tmp/first.log");
            root.addLog("/tmp/second.log");
            page.index = 0;
            backend.changes = [];
            root.saves = 0;
        }
        function choose(encoding) {
            mouseClick(inputEncoding);
            tryCompare(inputEncoding.popup, "opened", true);
            keyClick(Qt.Key_Home);
            for (let i = 0; i < root.encodingOptions.indexOf(encoding); ++i)
                keyClick(Qt.Key_Down);
            keyClick(Qt.Key_Return);
            tryCompare(inputEncoding.popup, "opened", false);
            compare(inputEncoding.currentText, encoding);
        }
        function test_new_logs_default_to_utf8() {
            compare(logs.get(0).encoding, "UTF-8");
            compare(logs.get(1).encoding, "UTF-8");
            compare(inputEncoding.currentText, "UTF-8");
            compare(backend.changes.length, 0);
            root.addLog("/tmp/saved.log", "file", "Saved", "CP932");
            page.index = 2;
            compare(inputEncoding.currentText, "CP932");
            compare(backend.changes.length, 0, "Restoring selection must not replay data");
        }
        function test_change_only_current_tab_and_save() {
            choose("CP932");
            compare(backend.changes.length, 1);
            compare(backend.changes[0], "CP932");
            compare(logs.get(0).encoding, "CP932");
            compare(logs.get(1).encoding, "UTF-8");
            compare(root.logConfiguration(logs.get(0)).encoding, "CP932");
            compare(root.saves, 1);
            root.addLog("/tmp/new.log");
            compare(logs.get(2).encoding, "UTF-8", "Selection must not affect new logs");
            choose("EUC-JP");
            compare(backend.changes[1], "EUC-JP");
            compare(logs.get(0).encoding, "EUC-JP");
        }
        function test_selection_tracks_external_edits_and_tab_moves() {
            choose("CP932");
            logs.setProperty(0, "encoding", "UTF-16LE");
            compare(inputEncoding.currentText, "UTF-16LE");
            page.index = 1;
            compare(inputEncoding.currentText, "UTF-8");
            choose("BIG5");
            compare(logs.get(0).encoding, "UTF-16LE");
            compare(logs.get(1).encoding, "BIG5");
            compare(backend.changes.length, 2);
        }
        function test_syslog_encoding_can_be_changed_and_saved() {
            logs.setProperty(0, "source", "syslog-udp");
            compare(inputEncoding.visible, true);
            compare(inputEncoding.currentText, "UTF-8");
            choose("CP932");
            compare(backend.changes[0], "CP932");
            compare(root.logConfiguration(logs.get(0)).encoding, "CP932");
            compare(root.saves, 1);
            compare(encodingControls.children[1].text, "→ UTF-8");
            page.index = 1;
            page.index = 0;
            compare(inputEncoding.currentText, "CP932");
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-encoding-") as directory:
    path = Path(directory) / "tst_EncodingControls.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)