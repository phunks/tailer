#!/usr/bin/env python3
"""Check production analysis button colors and actions."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                                Button {\n                                    id: analysisApplyButton")
end = source.index("\n                            }", start)
buttons = source[start:end]
fixture = """
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Control {
    id: root
    width: 400; height: 100
    function tr(key) { return key; }
    QtObject {
        id: analysisDialog
        property int applyCalls: 0
        function apply() { applyCalls++; }
    }
    QtObject {
        id: backend
        property int stopCalls: 0
        function stop_analysis() { stopCalls++; }
    }
    RowLayout {
        anchors.centerIn: parent
""" + buttons + """
    }
    TestCase {
        name: "AnalysisButtons"
        when: windowShown
        function test_colors_and_actions() {
            compare(analysisApplyButton.palette.button, "#1d4f1d");
            compare(analysisStopButton.palette.button, "#4f1916");
            compare(analysisApplyButton.palette.buttonText, "#ffffff");
            compare(analysisStopButton.palette.buttonText, "#ffffff");
            mouseClick(analysisApplyButton);
            compare(analysisDialog.applyCalls, 1);
            mouseClick(analysisStopButton);
            compare(backend.stopCalls, 1);
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-analysis-buttons-") as directory:
    path = Path(directory) / "tst_AnalysisButtons.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)