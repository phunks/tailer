#!/usr/bin/env python3
"""Check selection and right-click in the production search-result delegate."""

import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                            delegate: RowLayout {\n                                id: resultRow")
end = source.index("                            // WheelHandler", start)
delegate = source[start:end]
fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    width: 800; height: 300
    function tr(key) { return key; }
    QtObject { id: page; property string trapRulesJson: "[]" }
    QtObject {
        id: backend
        property int contextLine: -1
        function highlight_result(text) { return "<pre>" + text + "</pre>"; }
        function show_context(line) { contextLine = line; }
    }
    QtObject { id: follow; property bool checked: true }
    QtObject {
        id: selectionMenu
        property string selectedText: ""
        property var textItem: null
        property bool opened: false
        function popup() { opened = true; }
    }
    ListView {
        id: results
        anchors.fill: parent
        model: [{line: 42, text: "ERROR xyz aaaa"}]
""" + delegate + """
    }
    TestCase {
        name: "TrapResults"
        when: windowShown
        function test_selection_and_context() {
            tryVerify(() => results.itemAtIndex(0) !== null);
            const row = results.itemAtIndex(0);
            const area = row.children[1];
            compare(area.readOnly, true);
            verify(area.selectByMouse);
            area.select(0, 5);
            compare(area.selectedText, "ERROR");
            mouseClick(area, 25, 10, Qt.RightButton);
            compare(selectionMenu.opened, true);
            compare(selectionMenu.selectedText, "ERROR");
            compare(area.selectedText, "ERROR");
            compare(backend.contextLine, -1);
            mouseClick(row.children[0]);
            compare(backend.contextLine, 42);
            compare(follow.checked, false);
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-trap-results-") as directory:
    path = Path(directory) / "tst_TrapResults.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)