#!/usr/bin/env python3
"""Exercise the production tab appearance, close button and confirmation dialog."""
import os
from pathlib import Path
import subprocess
import tempfile

source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
start = source.rindex("    Dialog {", 0, source.index("        id: closeTabDialog"))
end = source.index("\n    Dialog {", start + 1)
tabs_start = source.index("        TabBar {\n            id: tabs")
tabs_end = source.index("\n        Label {", tabs_start)
close_tabs_start = source.index("    function closeTabs(")
close_tabs_end = source.index("\n    function restartTab(", close_tabs_start)
fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    visible: true
    width: 800; height: 600
    property int closed: 0
    property int closedIndex: -1
    property bool movingTab: false
    function tr(key) { return key; }
    function translateButtons(dialog) {}
    function scheduleSave() {}
    function restartTab(index) {}
    function registerBookmark(index) {}
    function dropTab(from, sceneX, sceneY) {}
    function closeTab(index) { closedIndex = index; closed++; logs.remove(index); }
    QtObject { id: editTabDialog; function edit(index) {} }
    ListModel { id: logs }
    ListModel { id: bookmarks }
""" + source[close_tabs_start:close_tabs_end] + source[start:end] + """
    ColumnLayout {
        anchors.fill: parent
""" + source[tabs_start:tabs_end] + """
    }
    TabButton {
        id: baseline
        visible: false
        rightPadding: 36
        background: Rectangle { implicitHeight: 21 }
        contentItem: RowLayout {
            spacing: 6
            Label { text: "First" }
            Rectangle { implicitWidth: 8; implicitHeight: 8 }
        }
    }
    TestCase {
        name: "CloseTab"
        when: windowShown
        function init() {
            root.closed = 0;
            root.closedIndex = -1;
            logs.clear();
            logs.append({logTitle: "First", unread: false, alert: false});
            logs.append({logTitle: "Second", unread: false, alert: false});
            tabs.currentIndex = 0;
            wait(50);
        }
        function cleanup() { closeTabDialog.close(); }
        function test_appearance() {
            const first = tabRepeater.itemAt(0);
            const second = tabRepeater.itemAt(1);
            compare(first.height, baseline.implicitHeight + 6);
            compare(second.height, first.height);
            compare(first.background.color.toString(), Qt.lighter(first.palette.button, 1.50).toString());
            verify(first.background.color.toString() !== second.background.color.toString());
            mouseClick(second, 10, second.height / 2);
            compare(tabs.currentIndex, 1);
            compare(second.background.color.toString(), Qt.lighter(second.palette.button, 1.50).toString());
            compare(first.background.color.toString(), first.palette.button.toString());
            compare(root.closed, 0);
        }
        function test_close_button() {
            const closeButton = findChild(tabRepeater.itemAt(1), "closeTabButton");
            verify(closeButton !== null);
            mouseClick(closeButton);
            tryCompare(closeTabDialog, "opened", true);
            compare(closeTabDialog.tabIndex, 1);
            compare(root.closed, 0);
            compare(logs.count, 2);
            mouseClick(closeTabDialog.standardButton(Dialog.Ok));
            tryCompare(root, "closed", 1);
            compare(root.closedIndex, 1);
            compare(logs.count, 1);
            compare(logs.get(0).logTitle, "First");
        }
        function test_invalid_index() {
            closeTabDialog.requestClose(-1);
            compare(closeTabDialog.visible, false);
            closeTabDialog.requestClose(logs.count);
            compare(closeTabDialog.visible, false);
            compare(root.closed, 0);
        }
        function test_confirmation() {
            closeTabDialog.requestClose(0);
            tryCompare(closeTabDialog, "opened", true);
            keyClick(Qt.Key_Escape);
            tryCompare(closeTabDialog, "visible", false);
            compare(root.closed, 0);
            compare(logs.count, 2);
            closeTabDialog.open();
            tryCompare(closeTabDialog, "opened", true);
            mouseClick(closeTabDialog.standardButton(Dialog.Cancel));
            tryCompare(closeTabDialog, "visible", false);
            compare(root.closed, 0);
            compare(logs.count, 2);
            closeTabDialog.open();
            tryCompare(closeTabDialog, "opened", true);
            mouseClick(closeTabDialog.standardButton(Dialog.Ok));
            tryCompare(root, "closed", 1);
        }
        function test_close_multiple_data() {
            return [
                {tag: "others", mode: "others", title: "Close other tabs?", remaining: 1, closed: 2},
                {tag: "all", mode: "all", title: "Close all tabs?", remaining: 0, closed: 3}
            ];
        }
        function test_close_multiple(data) {
            logs.append({logTitle: "Third", unread: false, alert: false});
            closeTabDialog.requestClose(1, data.mode);
            tryCompare(closeTabDialog, "opened", true);
            compare(closeTabDialog.title, data.title);
            compare(closeTabDialog.closeMode, data.mode);
            compare(root.closed, 0);
            mouseClick(closeTabDialog.standardButton(Dialog.Cancel));
            tryCompare(closeTabDialog, "visible", false);
            compare(root.closed, 0);
            compare(logs.count, 3);

            closeTabDialog.requestClose(1, data.mode);
            tryCompare(closeTabDialog, "opened", true);
            mouseClick(closeTabDialog.standardButton(Dialog.Ok));
            tryCompare(root, "closed", data.closed);
            compare(logs.count, data.remaining);
            if (data.mode === "others") {
                compare(logs.get(0).logTitle, "Second");
                // A subsequent single-close request must not reuse the bulk mode.
                closeTabDialog.requestClose(0);
                tryCompare(closeTabDialog, "opened", true);
                compare(closeTabDialog.closeMode, "single");
                compare(closeTabDialog.title, "Close log tab?");
                mouseClick(closeTabDialog.standardButton(Dialog.Ok));
                tryCompare(root, "closed", 3);
                compare(logs.count, 0);
            }
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-close-test-") as directory:
    path = Path(directory) / "tst_CloseTab.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)