#!/usr/bin/env python3
"""Drive production analysis moving, resizing and non-modal popup behavior."""
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                    Dialog {\n                        id: analysisDialog")
end = source.index("                        property bool initialized:", start)
dialog = source[start:end]
fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtTest
Item {
    id: root
    width: 1100; height: 720
    visible: true
    property int logClicks: 0
    function tr(key) { return key; }
    MouseArea { anchors.fill: parent; onClicked: root.logClicks++ }
""" + dialog + """
        contentItem: TextArea { text: "Editable analysis settings"; selectByMouse: true }
    }
    TestCase {
        name: "AnalysisDrag"
        when: windowShown
        function init() {
            analysisDialog.width = 900;
            analysisDialog.height = 680;
            analysisDialog.x = 100;
            analysisDialog.y = 20;
            root.logClicks = 0;
            analysisDialog.open();
            tryCompare(analysisDialog, "opened", true);
        }
        function cleanup() {
            analysisDialog.close();
            tryCompare(analysisDialog, "visible", false);
        }
        function dragResize(dx, dy) {
            const start = analysisResizeHandle.mapToItem(root,
                analysisResizeHandle.width / 2, analysisResizeHandle.height / 2);
            mousePress(root, start.x, start.y);
            for (let step = 1; step <= 8; ++step)
                mouseMove(root, start.x + dx * step / 8, start.y + dy * step / 8, 10);
            mouseRelease(root, start.x + dx, start.y + dy);
        }
        function test_resize_and_reopen() {
            dragResize(100, 20);
            compare(analysisDialog.width, 1000);
            compare(analysisDialog.height, 700);
            compare(analysisDialog.x, 100);
            compare(analysisDialog.y, 20);
            dragResize(-200, -180);
            compare(analysisDialog.width, 800);
            compare(analysisDialog.height, 520);
            analysisDialog.close();
            tryCompare(analysisDialog, "visible", false);
            analysisDialog.open();
            tryCompare(analysisDialog, "opened", true);
            compare(analysisDialog.width, 800);
            compare(analysisDialog.height, 520);
            mouseClick(root, 10, 10);
            compare(root.logClicks, 1);
            verify(analysisDialog.opened);
        }
        function test_resize_limits() {
            dragResize(-1000, -1000);
            compare(analysisDialog.width, analysisDialog.minimumResizeWidth);
            compare(analysisDialog.height, analysisDialog.minimumResizeHeight);
            dragResize(1500, 1500);
            compare(analysisDialog.width, analysisDialog.parent.width);
            compare(analysisDialog.height, analysisDialog.parent.height);
        }
        function test_resize_does_not_recenter() {
            analysisDialog.x = Qt.binding(() => (analysisDialog.parent.width - analysisDialog.width) / 2);
            analysisDialog.y = Qt.binding(() => (analysisDialog.parent.height - analysisDialog.height) / 2);
            const x = analysisDialog.x;
            const y = analysisDialog.y;
            dragResize(-100, -100);
            compare(analysisDialog.width, 800);
            compare(analysisDialog.height, 580);
            compare(analysisDialog.x, x);
            compare(analysisDialog.y, y);
        }
        function dragTitle(dx, dy) {
            const start = analysisTitleBar.mapToItem(root, 40, analysisTitleBar.height / 2);
            mousePress(root, start.x, start.y);
            for (let step = 1; step <= 8; ++step)
                mouseMove(root, start.x + dx * step / 8, start.y + dy * step / 8, 10);
            mouseRelease(root, start.x + dx, start.y + dy);
        }
        function test_move_and_read_logs() {
            analysisDialog.open();
            tryCompare(analysisDialog, "opened", true);
            const initialX = analysisDialog.x;
            const initialY = analysisDialog.y;
            dragTitle(180, 100);
            compare(analysisDialog.x, initialX + 180);
            compare(analysisDialog.y, initialY + 100);
            mouseClick(root, 10, 10);
            compare(root.logClicks, 1, "The underlying log must remain interactive");
            verify(analysisDialog.opened, "Clicking the log must not close analysis");
            dragTitle(1000, 1000);
            verify(analysisDialog.x <= analysisDialog.parent.width - 80);
            verify(analysisDialog.y <= analysisDialog.parent.height - analysisTitleBar.height);
            verify(analysisDialog.y >= 0);
            const x = analysisDialog.x;
            const y = analysisDialog.y;
            analysisDialog.close();
            tryCompare(analysisDialog, "visible", false);
            analysisDialog.open();
            tryCompare(analysisDialog, "opened", true);
            compare(analysisDialog.x, x);
            compare(analysisDialog.y, y);
            analysisDialog.forceActiveFocus();
            keyClick(Qt.Key_Escape);
            tryCompare(analysisDialog, "visible", false);
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-analysis-drag-") as directory:
    path = Path(directory) / "tst_AnalysisDrag.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)