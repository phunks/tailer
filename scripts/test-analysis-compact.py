#!/usr/bin/env python3
"""Exercise the production analysis dialog's compact view with Qt Quick Test."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.rfind("                    Dialog {", 0, source.index("id: analysisDialog"))
end = source.index("                    Menu {\n                        id: selectionMenu", start)
dialog = source[start:end]
fixture = """
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Control {
    id: root
    width: 1100; height: 720; visible: true
    function tr(key) { return key; }
    function scheduleSave() {}
    function translateButtons(dialog) {}
    QtObject {
        id: page
        property int index: 0
        property bool viewActive: true
        property bool analysisMergeFirstColumn: false
        property string analysisPresetId: ""
    }
    QtObject { id: logs; function setProperty(index, key, value) {} }
    QtObject { id: workspace; property var analysisPresets: [] }
    QtObject {
        id: backend
        property bool analysisBusy: false
        property var analysisResult: ({})
    }
""" + dialog + """
    TestCase {
        name: "AnalysisCompact"
        when: windowShown
        function init() {
            page.viewActive = true;
            analysisDialog.reopenOnActivation = false;
            analysisDialog.compact = false;
            analysisDialog.settingsError = "";
            analysisDialog.initialized = true;
            analysisDialog.width = 900;
            analysisDialog.height = 680;
            analysisDialog.x = 40;
            analysisDialog.y = 20;
            backend.analysisResult = {};
            analysisTextToggle.checked = false;
            analysisDialog.open();
            wait(100);
        }
        function cleanup() { analysisDialog.close(); }
        function results() {
            const rows = [];
            for (let i = 0; i < 100; ++i) rows.push({cells: ["path " + i, i, i, i]});
            backend.analysisResult = {columns: ["path", "count()", "min(n)", "max(n)"], rows: rows};
            wait(100);
        }
        function test_availability() {
            verify(!analysisCompactButton.enabled);
            results();
            verify(analysisCompactButton.enabled);
            backend.analysisResult = {error: "Compile error"};
            verify(!analysisCompactButton.enabled);
            results();
            analysisDialog.settingsError = "Invalid field mappings";
            verify(!analysisCompactButton.enabled);
        }
        function test_compact_restore_and_scroll() {
            results();
            analysisExtractionToggle.checked = true;
            analysisPreviewToggle.checked = true;
            analysisRegex.text = "preserved expression";
            analysisTextToggle.checked = true;
            mouseClick(analysisCompactButton);
            wait(150);
            verify(analysisDialog.compact);
            verify(analysisDialog.width < 900 && analysisDialog.height < 680);
            compare(analysisDialog.x, 40);
            compare(analysisDialog.y, 20);
            verify(!analysisApplyButton.visible);
            verify(!analysisExtractionSection.visible);
            verify(!analysisPreviewSection.visible);
            verify(!analysisResultToolbar.visible);
            verify(!analysisTextResults.visible);
            verify(analysisResultSection.visible);
            verify(analysisResults.height > 80);
            verify(analysisTable.contentHeight > analysisTable.height);
            verify(analysisHorizontalScroll.visible);
            analysisTable.contentY = 300;
            analysisTable.contentX = 100;
            wait(50);
            verify(analysisTable.contentY > 0 && analysisTable.contentX > 0);
            backend.analysisResult = {error: "Live analysis failed"};
            verify(analysisCompactButton.enabled, "Restoring must remain possible after an error");
            mouseClick(analysisCompactButton);
            wait(100);
            verify(!analysisDialog.compact);
            compare(analysisDialog.width, 900);
            compare(analysisDialog.height, 680);
            compare(analysisRegex.text, "preserved expression");
            verify(analysisExtractionToggle.checked);
            verify(analysisTextToggle.checked);
            verify(analysisTextResults.visible);
            verify(analysisDiagnostics.visible);
        }
        function test_compact_reopen_and_escape() {
            results();
            mouseClick(analysisCompactButton);
            wait(50);
            analysisDialog.close();
            analysisDialog.open();
            wait(100);
            verify(analysisDialog.compact);
            verify(analysisCompactButton.enabled);
            keyClick(Qt.Key_Escape);
            tryCompare(analysisDialog, "visible", false);
        }
        function test_tab_switch_restores_compact_state() {
            results();
            analysisRegex.text = "tab-specific settings";
            mouseClick(analysisCompactButton);
            wait(100);
            const savedWidth = analysisDialog.width;
            const savedHeight = analysisDialog.height;
            for (let i = 0; i < 3; ++i) {
                page.viewActive = false;
                tryCompare(analysisDialog, "visible", false);
                verify(analysisDialog.reopenOnActivation);
                page.viewActive = true;
                tryCompare(analysisDialog, "opened", true);
                verify(analysisDialog.compact);
                compare(analysisDialog.width, savedWidth);
                compare(analysisDialog.height, savedHeight);
                compare(analysisDialog.x, 40);
                compare(analysisDialog.y, 20);
                compare(analysisRegex.text, "tab-specific settings");
            }
        }
        function test_closed_dialog_stays_closed_after_tab_switch() {
            keyClick(Qt.Key_Escape);
            tryCompare(analysisDialog, "visible", false);
            page.viewActive = false;
            page.viewActive = true;
            wait(100);
            verify(!analysisDialog.visible);
            verify(!analysisDialog.reopenOnActivation);
        }
        function test_tab_switch_hides_preset_confirmation() {
            analysisPresetDelete.open();
            tryCompare(analysisPresetDelete, "opened", true);
            page.viewActive = false;
            tryCompare(analysisDialog, "visible", false);
            tryCompare(analysisPresetDelete, "visible", false);
            page.viewActive = true;
            tryCompare(analysisDialog, "opened", true);
            verify(!analysisPresetDelete.visible);
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-analysis-compact-") as directory:
    path = Path(directory) / "tst_AnalysisCompact.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)