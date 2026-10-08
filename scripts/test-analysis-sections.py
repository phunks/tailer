#!/usr/bin/env python3
"""Exercise production analysis disclosure controls and result sizing."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                            ToolButton {\n                                id: analysisExtractionToggle")
end = source.index('                            RowLayout {\n                                Label { text: root.tr("Result columns") }', start)
sections = source[start:end]
start = source.index("                            RowLayout {\n                                id: analysisResultToolbar", end)
end = source.index("\n                        }\n                      }", start)
results = source[start:end]
start = source.index("                          id: analysisContent")
end = source.index("                            Label {", start)
content_layout = source[start:end]

fixture = """
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Control {
    id: root
    width: 860; height: 760
    function tr(key) { return key; }
    QtObject { id: page; property bool analysisMergeFirstColumn: false }
    QtObject {
        id: backend
        property bool analysisBusy: false
    }
    QtObject {
        id: analysisDialog
        property string settingsError: ""
        function rememberMergeFirstColumn() { page.analysisMergeFirstColumn = analysisMergeFirstToggle.checked; }
        property var statistics: ({preview: [{method: "GET"}], columns: ["count"], rows: [{cells: [2]}]})
    }
    ScrollView {
        anchors.fill: parent
""" + content_layout + sections + results + """
        }
    }
    TestCase {
        name: "AnalysisSections"
        when: windowShown
        function init() {
            analysisMergeFirstToggle.checked = false;
            analysisResultSection.sortColumn = "";
            analysisResultSection.sortDescending = false;
            analysisResultSection.customColumnWidths = {};
            analysisTable.contentX = 0;
            analysisDialog.statistics = {preview: [{method: "GET"}], columns: ["count"], rows: [{cells: [2]}]};
            wait(50);
        }
        function dragColumn(index, dx) {
            const grip = findChild(analysisHeader, "analysisColumnResize_" + index);
            verify(grip !== null);
            const start = grip.mapToItem(root, 2, grip.height / 2);
            mousePress(root, start.x, start.y);
            verify(grip.pressed, "Column resize grip must receive the press at " + start.x + ", " + start.y);
            for (let step = 1; step <= 8; ++step)
                mouseMove(root, start.x + dx * step / 8, start.y, 10);
            mouseRelease(root, start.x + dx, start.y);
            wait(50);
        }
        function test_column_resizing() {
            analysisDialog.statistics = {columns: ["path", "count()"],
                rows: [{cells: ["/long/path", 2]}, {cells: ["/", 1]}]};
            wait(100);
            dragColumn(0, 200);
            compare(analysisResultSection.widths[0], 380);
            compare(findChild(analysisHeader, "analysisHeader_0").width, 380);
            compare(findChild(analysisTable, "analysisCell_0_0").width, 380);
            compare(analysisResultSection.widths[1], 180);
            compare(analysisResultSection.tableWidth, 560);
            compare(analysisResultSection.sortColumn, "", "Resizing must not trigger sorting");
            dragColumn(0, -100);
            compare(analysisResultSection.widths[0], 280);
            dragColumn(0, -250);
            compare(analysisResultSection.widths[0], analysisResultSection.minimumColumnWidth);
            compare(findChild(analysisTable, "analysisCell_0_0").width, 64);
            mouseClick(findChild(analysisHeader, "analysisHeader_1"));
            compare(analysisResultSection.sortColumn, "count()", "Header clicks must still sort");
            analysisDialog.statistics = {columns: ["count()", "path"], rows: [{cells: [3, "/new"]}]};
            wait(100);
            compare(analysisResultSection.widths[0], 180);
            compare(analysisResultSection.widths[1], 64, "Widths must follow column names on refresh/reorder");
            compare(findChild(analysisTable, "analysisCell_0_1").width, 64);
            mouseClick(analysisTextToggle);
            mouseClick(analysisTextToggle);
            compare(analysisResultSection.widths[1], 64);
        }
        function test_column_resizing_while_scrolled() {
            analysisDialog.statistics = {columns: ["path", "method", "status", "avg(ms)", "count()"],
                rows: [{cells: ["/", "GET", 200, 1.5, 2]}]};
            wait(100);
            analysisTable.contentX = 100;
            wait(50);
            dragColumn(4, 200);
            compare(analysisResultSection.widths[4], 380);
            compare(findChild(analysisTable, "analysisCell_0_4").width, 380);
            compare(analysisHeader.contentX, analysisTable.contentX);
            verify(analysisHorizontalScroll.visible);
            compare(analysisContent.contentItem.contentX, 0);
            analysisTable.contentX = analysisTable.contentWidth - analysisTable.width;
            wait(50);
            dragColumn(4, -300);
            compare(analysisResultSection.widths[4], 80);
            compare(analysisHeader.contentX, analysisTable.contentX);
            verify(analysisTable.contentX <= Math.max(0, analysisTable.contentWidth - analysisTable.width) + 1);
        }
        function test_merge_first_column() {
            analysisDialog.statistics = {columns: ["time", "count()"], rows: [
                {cells: ["2026-10-08", 2]}, {cells: ["2026-10-08", 1]},
                {cells: ["2026-10-09", 3]}, {cells: ["2026-10-08", 4]}]};
            wait(100);
            const first = findChild(analysisTable, "analysisCell_0_0");
            const repeated = findChild(analysisTable, "analysisCell_1_0");
            const otherColumn = findChild(analysisTable, "analysisCell_1_1");
            verify(findChild(repeated, "analysisCellLabel").visible);
            mouseClick(analysisMergeFirstToggle);
            wait(100);
            verify(!first.continuation);
            verify(repeated.continuation);
            verify(!findChild(repeated, "analysisCellLabel").visible);
            verify(!findChild(repeated, "analysisCellTopBorder").visible);
            verify(!findChild(first, "analysisCellBottomBorder").visible);
            verify(findChild(repeated, "analysisCellBottomBorder").visible);
            compare(first.color, repeated.color);
            verify(findChild(otherColumn, "analysisCellLabel").visible);
            verify(!findChild(analysisTable, "analysisCell_3_0").continuation);
            mouseMove(repeated, 50, 15);
            const tooltip = findChild(repeated, "analysisCellTooltip");
            tryCompare(tooltip, "visible", true);
            compare(tooltip.text, "2026-10-08");
            analysisResultSection.sortBy("time");
            wait(100);
            verify(findChild(analysisTable, "analysisCell_2_0").continuation);
            mouseClick(analysisTextToggle);
            wait(100);
            verify(!analysisMergeFirstToggle.enabled);
            compare(analysisResultText.text.split("2026-10-08").length - 1, 3);
            mouseClick(analysisTextToggle);
            mouseClick(analysisMergeFirstToggle);
            wait(100);
            const restored = findChild(analysisTable, "analysisCell_1_0");
            verify(findChild(restored, "analysisCellLabel").visible);
            compare(restored.border.width, 1);
            analysisMergeFirstToggle.checked = true;
            analysisResultSection.sortColumn = "";
            analysisDialog.statistics = {columns: ["value"], rows: [
                {cells: [1.0001]}, {cells: [1.0002]}, {cells: [1.0002]},
                {cells: ["1.0002"]}, {cells: [null]}, {cells: [null]}]};
            wait(100);
            verify(!findChild(analysisTable, "analysisCell_1_0").continuation);
            verify(findChild(analysisTable, "analysisCell_2_0").continuation);
            verify(!findChild(analysisTable, "analysisCell_3_0").continuation);
            verify(!findChild(analysisTable, "analysisCell_5_0").continuation);
        }
        function test_busy_indicator_does_not_shift_results() {
            wait(100);
            const toolbarHeight = analysisResultToolbar.height;
            const toggleX = analysisTextToggle.x;
            const resultY = analysisResultSection.y;
            const resultHeight = analysisResultSection.height;
            verify(!analysisResultBusy.visible);
            backend.analysisBusy = true;
            wait(100);
            verify(analysisResultBusy.visible && analysisResultBusy.running);
            compare(analysisResultToolbar.height, toolbarHeight);
            compare(analysisTextToggle.x, toggleX);
            compare(analysisResultSection.y, resultY);
            compare(analysisResultSection.height, resultHeight);
            backend.analysisBusy = false;
            wait(100);
            verify(!analysisResultBusy.visible);
            compare(analysisResultSection.y, resultY);
            compare(analysisResultSection.height, resultHeight);
        }
        function test_header_sorting() {
            const rows = [{cells: ["b", 10]}, {cells: ["a", 2]},
                {cells: ["c", 2]}, {cells: ["d", -1]}];
            analysisDialog.statistics = {columns: ["path", "count()"], rows: rows,
                samples: [{line: 9, reason: "bad", text: "original log"}]};
            wait(100);
            const numberHeader = findChild(analysisHeader, "analysisHeader_1");
            mouseClick(numberHeader);
            wait(100);
            compare(analysisTable.model.map(row => row.cells[0]).join(","), "d,a,c,b");
            compare(rows[0].cells[0], "b", "Sorting must not mutate the source");
            compare(analysisResultSection.sortColumn, "count()");
            mouseClick(numberHeader);
            wait(100);
            compare(analysisTable.model.map(row => row.cells[0]).join(","), "b,a,c,d");
            verify(analysisResultSection.sortDescending);
            mouseClick(numberHeader);
            wait(100);
            compare(analysisTable.model.map(row => row.cells[0]).join(","), "b,a,c,d");
            compare(analysisResultSection.sortColumn, "");
            mouseClick(findChild(analysisHeader, "analysisHeader_0"));
            wait(100);
            compare(analysisTable.model.map(row => row.cells[0]).join(","), "a,b,c,d");
            mouseClick(analysisTextToggle);
            wait(100);
            compare(analysisResultText.text.split("\n")[1], '"a"\t2');
            verify(analysisDiagnosticText.text.includes("#9 bad"));
            mouseClick(analysisTextToggle);
            // Full precision, stable ties, live refresh and missing values.
            analysisResultSection.sortBy("count()");
            analysisDialog.statistics = {columns: ["count()", "path"], rows: [
                {cells: [1.0002, "b"]}, {cells: [1.0001, "a"]}, {cells: [null, "missing"]}]};
            wait(100);
            compare(analysisTable.model[0].cells[1], "a");
            analysisResultSection.sortBy("count()");
            wait(100);
            compare(analysisTable.model[0].cells[1], "b");
            compare(analysisTable.model[2].cells[1], "missing");
            analysisDialog.statistics = {columns: ["other"], rows: [{cells: [2]}, {cells: [1]}]};
            wait(100);
            compare(analysisTable.model[0].cells[0], 2);
            analysisDialog.statistics = {columns: ["count()"], rows: []};
            wait(100);
            compare(analysisTable.count, 0);
        }
        function test_text_display_switch() {
            const longValue = "long value/".repeat(100);
            const rows = [];
            for (let i = 0; i < 100; ++i) rows.push({cells: [longValue, 1.23456789, i]});
            analysisDialog.statistics = {
                columns: ["path", "avg(ms)", "count()"], rows: rows,
                samples: [{line: 42, reason: "Invalid timestamp", text: "bad log"}]
            };
            wait(100);
            verify(!analysisTextToggle.checked);
            verify(analysisResultSection.visible);
            verify(!analysisTextResults.visible);
            mouseClick(analysisTextToggle);
            tryCompare(analysisTextResults, "visible", true);
            verify(!analysisResultSection.visible);
            verify(analysisDiagnostics.visible);
            compare(analysisResultText.text.split("\n")[0], "path\tavg(ms)\tcount()");
            compare(analysisResultText.text.split("\n")[1], JSON.stringify(longValue) + "\t1.23456789\t0");
            verify(analysisResultText.readOnly && analysisResultText.selectByMouse);
            analysisResultText.selectAll();
            compare(analysisResultText.selectedText, analysisResultText.text);
            const view = analysisTextResults.contentItem;
            view.contentX = 100;
            view.contentY = 200;
            wait(100);
            const x = view.contentX;
            const y = view.contentY;
            analysisDialog.statistics = {columns: ["path", "avg(ms)", "count()"],
                rows: rows.concat([{cells: ["new", 2, 1]}]),
                samples: [{line: 42, reason: "Invalid timestamp", text: "bad log"}]};
            wait(100);
            compare(view.contentX, x);
            compare(view.contentY, y);
            verify(!analysisContent.ScrollBar.horizontal.visible);
            mouseClick(analysisTextToggle);
            tryCompare(analysisResultSection, "visible", true);
            verify(!analysisTextResults.visible);
            verify(analysisDiagnostics.visible);
            compare(analysisTable.count, 101);
        }
        function test_horizontal_scroll_is_table_local() {
            const columns = ["path", "avg(duration_ms)", "max(duration_ms)", "min(duration_ms)", "count()"];
            analysisDialog.statistics = {columns: columns, rows: [{cells: ["/", 1.1, 2.2, 0.1, 10]}]};
            wait(100);
            verify(analysisHorizontalScroll.visible);
            verify(analysisHorizontalScroll.size < 1);
            verify(!analysisContent.ScrollBar.horizontal.visible);
            compare(analysisContent.contentWidth, analysisContent.availableWidth);
            verify(analysisResultSection.width <= analysisContent.availableWidth);
            const bar = analysisHorizontalScroll;
            const startX = bar.width * bar.size / 2;
            mousePress(bar, startX, bar.height / 2);
            mouseMove(bar, bar.width - 2, bar.height / 2, 50);
            mouseRelease(bar, bar.width - 2, bar.height / 2);
            wait(100);
            verify(analysisTable.contentX > 0, "Dragging the table scrollbar must scroll the table");
            compare(analysisHeader.contentX, analysisTable.contentX);
            compare(analysisContent.contentItem.contentX, 0);
            verify(analysisTable.contentX + analysisTable.width >= analysisResultSection.tableWidth - 2,
                "The last column must be reachable");
            analysisDialog.statistics = {columns: ["count()"], rows: [{cells: [10]}]};
            wait(100);
            verify(!analysisHorizontalScroll.visible);
        }
        function test_disclosures_and_result_space() {
            verify(!analysisExtractionSection.visible);
            verify(!analysisRotoSection.visible);
            verify(!analysisPreviewSection.visible);
            verify(analysisUseRoto.checked);
            wait(50);
            const collapsedHeight = analysisResults.height;
            verify(collapsedHeight > 180);
            analysisRegex.text = "(?P<method>GET)";
            analysisMappings.text = '[{"name":"method","source":"method"}]';
            analysisScript.text = "saved script";

            mouseClick(analysisExtractionToggle);
            tryCompare(analysisExtractionSection, "visible", true);
            wait(50);
            verify(analysisRegex.visible && analysisMappings.visible);
            analysisFormat.currentIndex = 1;
            verify(analysisDelimiter.visible && !analysisRegex.visible);
            mouseClick(analysisExtractionToggle);
            tryCompare(analysisExtractionSection, "visible", false);
            compare(analysisFormat.currentIndex, 1);
            compare(analysisRegex.text, "(?P<method>GET)");
            compare(analysisMappings.text, '[{"name":"method","source":"method"}]');

            mouseClick(analysisRotoToggle);
            tryCompare(analysisRotoSection, "visible", true);
            wait(50);
            verify(analysisUseRoto.x > analysisRotoToggle.x + analysisRotoToggle.width);
            mouseClick(analysisUseRoto);
            verify(!analysisRotoSection.visible);
            verify(analysisRotoToggle.checked);
            wait(50);
            mouseClick(analysisUseRoto);
            verify(analysisRotoSection.visible);
            wait(50);
            mouseClick(analysisRotoToggle);
            verify(analysisUseRoto.checked);
            compare(analysisScript.text, "saved script");

            mouseClick(analysisPreviewToggle);
            tryCompare(analysisPreviewSection, "visible", true);
            wait(50);
            verify(analysisResults.height < collapsedHeight);
            mouseClick(analysisPreviewToggle);
            tryCompare(analysisResults, "height", collapsedHeight);
            const rows = [];
            for (let i = 0; i < 1000; ++i) rows.push({cells: [i]});
            analysisDialog.statistics = {columns: ["count"], rows: rows};
            wait(100);
            compare(analysisResults.height, collapsedHeight, "Result rows must not grow the viewport");
            verify(analysisResults.y + analysisResults.height <= analysisContent.availableHeight + 1);
            verify(analysisContent.contentHeight <= analysisContent.availableHeight + 1);
            verify(analysisResults.contentHeight > analysisResults.availableHeight);
            verify(analysisResults.ScrollBar.vertical.visible);
            analysisResults.ScrollBar.vertical.position = 0.5;
            wait(50);
            verify(analysisResults.contentItem.contentY > 0, "Results must scroll inside their viewport");
            analysisTable.forceActiveFocus();
            const view = analysisResults.contentItem;
            const savedY = view.contentY;
            const wideRows = [];
            for (let i = 0; i < 1100; ++i)
                wideRows.push({cells: [i, "long result value ".repeat(30)]});
            const wideColumns = ["count", "value", "method", "status", "min", "max"];
            analysisDialog.statistics = {columns: wideColumns, rows: wideRows};
            wait(100);
            compare(view.contentY, savedY, "A refreshed result must retain vertical position");
            view.contentX = 200;
            const savedX = view.contentX;
            for (let update = 0; update < 3; ++update) {
                analysisDialog.statistics = {columns: wideColumns, rows: wideRows.concat([{cells: [update, "new"]}])};
                wait(100);
                compare(view.contentY, savedY);
                compare(view.contentX, savedX, "Refresh must also retain horizontal position");
            }
            analysisDialog.statistics = {columns: ["count"], rows: rows.slice(0, 60)};
            wait(100);
            verify(view.contentY > 0, "Shorter results must clamp to the remaining range, not reset to top");
            verify(view.contentY <= view.originY + view.contentHeight - view.height + 1);
            compare(view.contentX, view.originX);
            analysisDialog.statistics = {columns: ["count"], rows: []};
            wait(100);
            compare(view.contentY, view.originY);
        }
        function test_cells_tooltips_and_diagnostics() {
            const longValue = "<original & value> " + "long path/".repeat(80);
            analysisDialog.statistics = {
                columns: ["path", "avg(duration_ms)", "count()"],
                rows: [{cells: [longValue, 1503.1053011422257, 39659]}],
                samples: [{line: 42, reason: "Invalid timestamp", text: "bad log line"}]
            };
            wait(100);
            verify(analysisDiagnostics.visible);
            verify(analysisDiagnosticText.text.includes("#42 Invalid timestamp"));
            verify(analysisDiagnosticText.text.includes("bad log line"));
            const cell = findChild(analysisTable, "analysisCell_0_0");
            verify(cell !== null);
            compare(cell.fullText, longValue);
            verify(cell.width <= 300);
            mouseMove(cell, cell.width / 2, cell.height / 2);
            const tooltip = findChild(cell, "analysisCellTooltip");
            tryCompare(tooltip, "visible", true);
            compare(tooltip.text, longValue);
            compare(tooltip.contentItem.text, longValue);
            verify(tooltip.width <= 600);
            const numeric = findChild(analysisTable, "analysisCell_0_1");
            compare(numeric.fullText, "1503.1053011422257");
            compare(analysisHeader.contentX, analysisTable.contentX);
            analysisDialog.statistics = {error: "Compile error at line 3"};
            wait(100);
            verify(analysisDiagnostics.visible);
            compare(analysisDiagnosticText.text, "Compile error at line 3");
            analysisDialog.statistics = {columns: ["count()"], rows: []};
            wait(100);
            verify(!analysisDiagnostics.visible);
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-analysis-sections-") as directory:
    path = Path(directory) / "tst_AnalysisSections.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)