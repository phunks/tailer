#!/usr/bin/env python3
"""Drive actual mouse drags and context menus against the production TabBar."""
import os
from pathlib import Path
import subprocess
import tempfile

source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
start = source.index("        TabBar {\n            id: tabs")
end = source.index("\n        Label {", start)
functions_start = source.index("    function moveTab(")
functions_end = source.index("\n    function restartTab(", functions_start)
fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    width: 900
    height: 300
    property bool movingTab: false
    function tr(key) { return key; }
    function scheduleSave() {}
    function registerBookmark(index) {}
    function restartTab(index) {}
    function closeTab(index) { logs.remove(index); }
    QtObject { id: closeTabDialog; property int tabIndex: -1; property string mode: "";
        function requestClose(index, closeMode = "single") { tabIndex = index; mode = closeMode; }
    }
    QtObject { id: editTabDialog; function edit(index) {} }
    ListModel { id: bookmarks }
    ListModel {
        id: logs
        ListElement { logTitle: "First"; unread: false; alert: false }
        ListElement { logTitle: "Second"; unread: false; alert: false }
        ListElement { logTitle: "Third"; unread: false; alert: false }
    }
    Repeater { id: pageRepeater; model: logs; Item { property var backend: ({connected:true}) } }
""" + source[functions_start:functions_end] + """
    ColumnLayout {
        anchors.fill: parent
""" + source[start:end] + """
        Item { Layout.fillHeight: true }
    }
    TestCase {
        name: "TabDragMouse"
        when: windowShown
        function test_drag_and_context_menu() {
            tryCompare(tabRepeater, "count", 3);
            tabs.currentIndex = 1;
            const first = tabRepeater.itemAt(0);
            const third = tabRepeater.itemAt(2);
            const target = third.mapToItem(root, third.width / 2, third.height / 2);
            const origin = first.mapToItem(root, first.width / 3, first.height / 2);
            mousePress(root, origin.x, origin.y, Qt.LeftButton);
            for (let step = 1; step <= 8; ++step)
                mouseMove(root, origin.x + (target.x - origin.x) * step / 8, origin.y, 20);
            mouseRelease(root, target.x, target.y, Qt.LeftButton);
            tryCompare(logs.get(2), "logTitle", "First");
            compare(logs.get(0).logTitle, "Second");
            compare(tabs.currentIndex, 0);
            const last = tabRepeater.itemAt(2);
            mouseClick(last, last.width / 3, last.height / 2, Qt.RightButton);
            tryCompare(last.contextMenu, "opened", true);
            let otherAction = null;
            for (let i = 0; i < last.contextMenu.count; ++i) {
                const item = last.contextMenu.itemAt(i);
                if (item && item.text === "Close other tabs") otherAction = item;
            }
            verify(otherAction !== null);
            mouseClick(otherAction, otherAction.width / 2, otherAction.height / 2);
            compare(closeTabDialog.tabIndex, 2);
            compare(closeTabDialog.mode, "others");
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-tab-drag-") as directory:
    path = Path(directory) / "tst_TabDrag.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)