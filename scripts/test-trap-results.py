#!/usr/bin/env python3
"""Check selection, context, and streaming scroll stability in search results."""

import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                        ListView {\n                            id: searchResults")
end = source.index("\n                    }", start)
results_view = source[start:end]
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
        property var results: [{line: 42, text: "ERROR xyz aaaa"}]
        signal changed()
        onResultsChanged: changed()
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
    ColumnLayout {
        anchors.fill: parent
""" + results_view + """
    }
    TestCase {
        name: "TrapResults"
        when: windowShown
        function init() {
            backend.results = [{line: 42, text: "ERROR xyz aaaa"}];
            follow.checked = true;
            selectionMenu.opened = false;
            wait(50);
        }
        function test_streaming_results_preserve_scroll_and_selection() {
            let rows = Array.from({length: 100}, (_, i) => ({line: i + 1, text: "ERROR row " + i}));
            backend.results = rows;
            wait(50);
            searchResults.positionViewAtIndex(40, ListView.Beginning);
            wait(50);
            const row = searchResults.itemAtIndex(40);
            verify(row !== null);
            const area = row.children[1];
            area.select(0, 5);
            const offset = row.y - searchResults.contentY;
            const y = searchResults.contentY;
            backend.changed();
            wait(20);
            compare(searchResults.contentY, y);
            compare(area.selectedText, "ERROR");
            for (let i = 0; i < 10; ++i) {
                rows = rows.concat([{line: 101 + i, text: "ERROR appended " + i}]);
                backend.results = rows;
                wait(20);
                compare(searchResults.contentY, y);
                compare(searchResults.itemAtIndex(40), row);
                compare(area.selectedText, "ERROR");
            }
            rows = rows.slice(10);
            backend.results = rows;
            wait(50);
            compare(searchResults.itemAtIndex(30).modelData.line, 41);
            compare(searchResults.itemAtIndex(30).y - searchResults.contentY, offset);
            compare(searchResults.itemAtIndex(30), row);
            compare(area.selectedText, "ERROR");
            backend.results = [{line: 2000, text: "Different search"}];
            wait(50);
            compare(searchResultsModel.count, 1);
            compare(searchResults.contentY, searchResults.originY);
            backend.results = [];
            wait(50);
            compare(searchResultsModel.count, 0);
            compare(searchResults.visible, false);
        }
        function test_selection_and_context() {
            tryVerify(() => searchResults.itemAtIndex(0) !== null);
            const row = searchResults.itemAtIndex(0);
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
        function test_scrollbar_is_always_visible_and_draggable() {
            const bar = searchResults.ScrollBar.vertical;
            compare(bar.policy, ScrollBar.AlwaysOn);
            verify(bar.visible);
            backend.results = Array.from({length: 200}, (_, i) => ({line: i + 1, text: "ERROR row " + i}));
            wait(50);
            verify(bar.visible);
            verify(bar.interactive);
            verify(bar.size < 1);
            const y = searchResults.contentY;
            const thumbY = bar.height * (bar.visualPosition + bar.visualSize / 2);
            mousePress(bar, bar.width / 2, thumbY);
            mouseMove(bar, bar.width / 2, bar.height * 0.7, 100);
            mouseRelease(bar, bar.width / 2, bar.height * 0.7);
            tryVerify(() => searchResults.contentY > y);
            const row = searchResults.itemAtIndex(searchResults.indexAt(1, searchResults.contentY + 1));
            verify(row !== null);
            verify(row.width <= searchResults.width - bar.width);
        }
        function test_long_results_keep_text_inside_viewport() {
            backend.results = Array.from({length: 80}, (_, i) => ({line: i + 1,
                text: "ERROR " + "long-log-".repeat(500)}));
            wait(50);
            searchResults.positionViewAtBeginning();
            wait(50);
            const row = searchResults.itemAtIndex(0);
            verify(row !== null);
            const area = row.children[1];
            verify(area.width > 0);
            verify(area.x + area.width <= row.width + 1, "Long lines must not expand the row beyond the viewport");
            searchResults.positionViewAtIndex(60, ListView.Beginning);
            wait(50);
            for (let step = 0; step < 8; ++step) {
                root.width = step % 2 === 0 ? 420 : 800;
                backend.results = backend.results.slice(2).concat([{line: 81 + step, text: "ERROR short"}]);
                wait(30);
                verify(searchResults.contentX === 0, "Horizontal position: " + searchResults.contentX);
                verify(searchResults.contentY >= searchResults.originY - 1, "Above origin: " + searchResults.contentY + " / " + searchResults.originY);
                verify(searchResults.contentY <= searchResults.originY
                    + Math.max(0, searchResults.contentHeight - searchResults.height) + 1, "Below end: " + searchResults.contentY);
                const index = searchResults.indexAt(1, searchResults.contentY + 1);
                verify(index >= 0, "A visible result must exist without clicking to repair layout");
                const visibleRow = searchResults.itemAtIndex(index);
                verify(visibleRow !== null);
                verify(visibleRow.children[1].width > 0, "Text area has zero width");
                verify(visibleRow.children[1].x + visibleRow.children[1].width <= visibleRow.width + 1, "Text area extends beyond row");
            }
            root.width = 800;
        }
        function test_results_shrinking_clamp_to_visible_rows() {
            backend.results = Array.from({length: 100}, (_, i) => ({line: i + 1, text: "ERROR row " + i}));
            wait(50);
            searchResults.positionViewAtIndex(55, ListView.Beginning);
            wait(50);
            backend.results = backend.results.slice(0, 56);
            wait(50);
            verify(searchResults.contentY <= searchResults.originY
                + Math.max(0, searchResults.contentHeight - searchResults.height) + 1,
                "Restored anchor must not place the viewport beyond the results");
            verify(searchResults.indexAt(1, searchResults.contentY + searchResults.height / 2) >= 0);
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