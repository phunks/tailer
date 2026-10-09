#!/usr/bin/env python3
"""Exercise actual mouse input against the production bookmark list delegate."""
import os
from pathlib import Path
import subprocess
import tempfile


source = (Path(__file__).resolve().parent.parent / "src/Main.qml").read_text()
start = source.index("            ListView {\n                id: bookmarkList")
end = source.index("\n            Label {", start)
fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    width: 620
    height: 480
    property int openedIndex: -1
    property int editedIndex: -1
    property int deletedIndex: -1
    function tr(key) { return key; }
    property string connectionName: "Production"
    function profileById(id) { return {name: root.connectionName}; }
    function openBookmark(index) { openedIndex = index; bookmarksDialog.close(); }
    function editBookmark(index) { editedIndex = index; }
    function deleteBookmark(index) { deletedIndex = index; bookmarks.remove(index); }
    property int openedGroup: -1
    function openBookmarkGroup(id) { openedGroup = 1; bookmarksDialog.close(); }
    function editBookmarkGroup(id, creating) { editedIndex = 10; }
    function deleteBookmarkGroup(id) { deletedIndex = 10; }
    function toggleBookmarkGroup(id) { bookmarkTree.setProperty(0, "expanded", !bookmarkTree.get(0).expanded); }
    ListModel { id: bookmarkGroups }
    ListModel {
        id: bookmarkTree
        ListElement { isGroup: true; nodeId: "prod"; sourceIndex: 0; depth: 0; expanded: true; bookmarkTitle: "Production"; logPath: ""; source: ""; remote: false; connectionId: "" }
        ListElement { isGroup: false; nodeId: ""; sourceIndex: 0; depth: 1; expanded: false; bookmarkTitle: "First log"; logPath: "/tmp/first"; source: "file"; remote: false; connectionId: "" }
        ListElement { isGroup: false; nodeId: ""; sourceIndex: 1; depth: 1; expanded: false; bookmarkTitle: "Second log"; logPath: "/tmp/second"; source: "file"; remote: true; connectionId: "prod" }
    }
    ListModel {
        id: bookmarks
        ListElement { bookmarkTitle: "First log"; logPath: "/tmp/first"; remote: false; connectionId: "" }
        ListElement { bookmarkTitle: "Second log"; logPath: "/tmp/second"; remote: true; connectionId: "prod" }
    }
    Dialog {
        id: bookmarksDialog
        width: 580
        height: 400
        anchors.centerIn: parent
        modal: true
        ColumnLayout {
            anchors.fill: parent
""" + source[start:end] + """
        }
    }
    TestCase {
        name: "BookmarkListMouse"
        when: windowShown
        function test_single_line_columns_and_tooltip() {
            bookmarksDialog.open();
            tryCompare(bookmarksDialog, "opened", true);
            tryVerify(() => bookmarkList.itemAtIndex(2) !== null);
            const first = bookmarkList.itemAtIndex(1);
            const second = bookmarkList.itemAtIndex(2);
            const title = findChild(second, "bookmarkTitleLabel");
            const connection = findChild(second, "bookmarkConnectionLabel");
            const path = findChild(second, "bookmarkPathLabel");
            compare(connection.text, "Production");
            compare(findChild(first, "bookmarkConnectionLabel").text, "");
            const originalHeight = second.height;
            const longTitle = "<b>Very long bookmark name</b> ".repeat(20);
            const longPath = "/var/log/" + "very-long-directory/".repeat(30) + "application.log";
            const longConnection = "Production SSH connection ".repeat(20);
            try {
                root.connectionName = longConnection;
                bookmarkTree.setProperty(2, "bookmarkTitle", longTitle);
                bookmarkTree.setProperty(2, "logPath", longPath);
                bookmarkTree.setProperty(2, "depth", 2);
                tryCompare(title, "text", longTitle);
                tryCompare(connection, "text", longConnection);
                tryCompare(path, "text", "file:" + longPath);
                tryVerify(() => title.truncated && connection.truncated && path.truncated);
                compare(second.height, originalHeight);
                compare(title.maximumLineCount, 1);
                compare(title.textFormat, Text.PlainText);
                compare(title.y, connection.y);
                compare(connection.y, path.y);
                const firstConnection = findChild(first, "bookmarkConnectionLabel");
                const firstPath = findChild(first, "bookmarkPathLabel");
                tryVerify(() => Math.round(connection.mapToItem(bookmarkList, 0, 0).x)
                    === Math.round(firstConnection.mapToItem(bookmarkList, 0, 0).x));
                tryVerify(() => Math.round(path.mapToItem(bookmarkList, 0, 0).x)
                    === Math.round(firstPath.mapToItem(bookmarkList, 0, 0).x));
                mouseMove(second, second.width / 2, second.height / 2);
                const tooltip = findChild(second, "bookmarkRowTooltip");
                tryCompare(tooltip, "visible", true);
                compare(tooltip.text, longTitle + "\\n" + longConnection + "\\nfile:" + longPath);
                compare(tooltip.contentItem.textFormat, Text.PlainText);
                verify(tooltip.width <= 600);
            } finally {
                root.connectionName = "Production";
                bookmarkTree.setProperty(2, "bookmarkTitle", "Second log");
                bookmarkTree.setProperty(2, "logPath", "/tmp/second");
                bookmarkTree.setProperty(2, "depth", 1);
                mouseMove(root, 0, 0);
                bookmarksDialog.close();
                tryCompare(bookmarksDialog, "visible", false);
            }
        }
        function test_source_prefix() {
            bookmarksDialog.open();
            tryCompare(bookmarksDialog, "opened", true);
            tryVerify(() => bookmarkList.itemAtIndex(2) !== null);
            const row = bookmarkList.itemAtIndex(2);
            const path = findChild(row, "bookmarkPathLabel");
            try {
                for (const entry of [
                    ["docker", "litellm-db", "docker:litellm-db"],
                    ["file", "/var/log/app.log", "file:/var/log/app.log"],
                    ["custom", "printf 'hello'", "custom:printf 'hello'"],
                    ["syslog-udp", "127.0.0.1:1514", "syslog-udp:127.0.0.1:1514"],
                    ["", "/tmp/legacy.log", "file:/tmp/legacy.log"]
                ]) {
                    bookmarkTree.setProperty(2, "source", entry[0]);
                    bookmarkTree.setProperty(2, "logPath", entry[1]);
                    tryCompare(path, "text", entry[2]);
                    compare(findChild(row, "bookmarkRowTooltip").text,
                            "Second log\\nProduction\\n" + entry[2]);
                }
            } finally {
                bookmarkTree.setProperty(2, "source", "file");
                bookmarkTree.setProperty(2, "logPath", "/tmp/second");
                bookmarksDialog.close();
                tryCompare(bookmarksDialog, "visible", false);
            }
        }
        function test_mouse_actions() {
            bookmarksDialog.open();
            tryCompare(bookmarksDialog, "opened", true);
            tryVerify(() => bookmarkList.itemAtIndex(2) !== null);
            const row = bookmarkList.itemAtIndex(2);
            verify(row !== null);
            const mouseArea = findChild(row, "bookmarkRowMouse");
            verify(mouseArea !== null);
            mouseClick(mouseArea, 20, 20, Qt.RightButton);
            tryCompare(row.contextMenu, "opened", true);
            compare(bookmarkList.currentIndex, 2);
            const edit = row.contextMenu.itemAt(2);
            compare(row.contextMenu.itemAt(0).height, 0);
            compare(row.contextMenu.itemAt(1).height, 0);
            compare(edit.y, 0);
            mouseClick(edit, edit.width / 2, edit.height / 2);
            compare(root.editedIndex, 1);
            tryCompare(row.contextMenu, "visible", false);
            mouseDoubleClickSequence(mouseArea, 20, 20, Qt.LeftButton);
            compare(root.openedIndex, 1);
            tryCompare(bookmarksDialog, "visible", false);
            bookmarksDialog.open();
            tryCompare(bookmarksDialog, "opened", true);
            tryVerify(() => bookmarkList.itemAtIndex(1) !== null);
            const first = bookmarkList.itemAtIndex(1);
            mouseClick(findChild(first, "bookmarkRowMouse"), 20, 20, Qt.RightButton);
            tryCompare(first.contextMenu, "opened", true);
            const remove = first.contextMenu.itemAt(3);
            mouseClick(remove, remove.width / 2, remove.height / 2);
            compare(root.deletedIndex, 0);
            compare(bookmarks.count, 1);
            const group = bookmarkList.itemAtIndex(0);
            const groupMouse = findChild(group, "bookmarkRowMouse");
            mouseClick(groupMouse, 80, 20, Qt.RightButton);
            tryCompare(group.contextMenu, "opened", true);
            verify(group.contextMenu.itemAt(0).height > 0);
            verify(group.contextMenu.itemAt(1).height > 0);
            group.contextMenu.close();
            tryCompare(group.contextMenu, "visible", false);
            mouseClick(groupMouse, 10, 20, Qt.LeftButton);
            compare(bookmarkTree.get(0).expanded, false);
            mouseDoubleClickSequence(groupMouse, 80, 20, Qt.LeftButton);
            compare(root.openedGroup, 1);
            tryCompare(bookmarksDialog, "visible", false);
        }
    }
}
"""
with tempfile.TemporaryDirectory(prefix="tailer-bookmark-list-") as directory:
    path = Path(directory) / "tst_BookmarkList.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)