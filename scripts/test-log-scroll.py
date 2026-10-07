#!/usr/bin/env python3
"""Exercise the actual log ScrollView with Qt Quick Test (Qt tools on PATH)."""

import os
from pathlib import Path
import subprocess
import tempfile


root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                        ScrollView {\n                            id: scroll")
end = source.index("                        Label {\n                            text: backend.status", start)
scroll_view = source[start:end]
follow_start = source.rindex("                            Button {", 0, source.index("                                id: follow"))
follow_end = source.index("                            ToolButton {", follow_start)
follow_button = source[follow_start:follow_end]

# Extract the production component so the regression check cannot silently drift
# to a separate copy of the wheel handler. Only its data dependencies are mocked.
fixture = """
import QtQuick
import QtQuick.Window
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest

Item {
    id: root
    width: 800
    height: 600
    function tr(key) { return key; }
    QtObject { id: page; property bool follow: true; function saveView() {} }
    QtObject { id: selectionMenu; property string selectedText: ""; property var textItem: null; function popup() {} }
    QtObject {
        id: backend
        property int selectedPosition: -1
        signal changed()
        onHighlightedTextChanged: changed()
        onSelectedPositionChanged: changed()
        property string highlightedText: Array.from({length: 300}, (_, i) => "Line " + i).join("<br>")
    }
    ColumnLayout {
        anchors.fill: parent
""" + follow_button + scroll_view + """
    }
    TestCase {
        id: testCase
        name: "LogScroll"
        when: windowShown
        property bool watchingFollow: false
        property int offBottomFrames: 0
        property bool watchingPaused: false
        property real pausedY: 0
        property real pausedX: 0
        property int shiftedPausedFrames: 0
        Connections {
            target: root.Window.window
            function onAfterAnimating() {
                if (testCase.watchingFollow && Math.abs(scroll.contentItem.contentY
                        - Math.max(0, scroll.contentItem.contentHeight - scroll.contentItem.height)) > 2)
                    ++testCase.offBottomFrames;
                if (testCase.watchingPaused && (Math.abs(scroll.contentItem.contentY - testCase.pausedY) > 1
                        || Math.abs(scroll.contentItem.contentX - testCase.pausedX) > 1))
                    ++testCase.shiftedPausedFrames;
            }
        }
        function initTestCase() { backend.changed(); wait(100); }
        function test_paused_view_does_not_jump_during_updates() {
            const before = backend.highlightedText;
            follow.checked = false;
            const lines = Array.from({length: 300}, (_, i) => "Line " + i + " " + "wide log ".repeat(40));
            backend.highlightedText = "<pre>" + lines.join("<br>") + "</pre>";
            wait(50);
            scroll.contentItem.contentY = 1200;
            scroll.contentItem.contentX = 120;
            pausedY = scroll.contentItem.contentY;
            pausedX = scroll.contentItem.contentX;
            verify(pausedY > 0 && pausedX > 0);
            shiftedPausedFrames = 0;
            watchingPaused = true;
            try {
                for (let i = 0; i < 12; ++i) {
                    lines.push("Appended " + i);
                    backend.highlightedText = "<pre>" + lines.join("<br>") + "</pre>";
                    compare(scroll.contentItem.contentY, pausedY, "No intermediate vertical reset");
                    compare(scroll.contentItem.contentX, pausedX, "No intermediate horizontal reset");
                    wait(i % 2 === 0 ? 20 : 80);
                    compare(scroll.contentItem.contentY, pausedY);
                    compare(scroll.contentItem.contentX, pausedX);
                }
                compare(shiftedPausedFrames, 0, "Every frame must keep the paused viewport stable");
            } finally {
                watchingPaused = false;
                backend.highlightedText = before;
                wait(50);
                follow.checked = true;
            }
        }
        function test_follow_does_not_jump_during_document_replacement() {
            const before = backend.highlightedText;
            follow.checked = true;
            wait(50);
            offBottomFrames = 0;
            watchingFollow = true;
            try {
                // Both growing output and a capped tail window replace the HTML.
                // Uneven waits simulate fragmented/batched SSH reception.
                for (let i = 0; i < 12; ++i) {
                    backend.highlightedText = "<pre>" + Array.from(
                        {length: 300 + i % 3}, (_, n) => "Line " + (n + i)).join("<br>") + "</pre>";
                    compare(scroll.contentItem.contentY,
                        Math.max(0, scroll.contentItem.contentHeight - scroll.contentItem.height),
                        "Restore follow before returning to the event loop");
                    wait(i % 2 === 0 ? 20 : 80);
                }
                compare(offBottomFrames, 0, "No rendered frame may jump away from the bottom");
            } finally {
                watchingFollow = false;
                backend.highlightedText = before;
                wait(50);
            }
        }
        function test_selection_survives_stream_updates() {
            const before = backend.highlightedText;
            follow.checked = false;
            scroll.contentItem.contentY = 0;
            backend.highlightedText = "<pre>先頭\n日本語 ERROR 障害\n末尾</pre>";
            wait(50);
            const start = logText.getText(0, logText.length).indexOf("日本語");
            logText.select(start, start + 3);
            compare(logText.selectedText, "日本語");
            const visible = logText.text;
            for (let i = 0; i < 3; ++i) {
                backend.highlightedText = "<pre>先頭\n日本語 ERROR 障害\n末尾\n追加" + i + "</pre>";
                wait(30);
                compare(logText.text, visible, "Keep the selected document stable");
                compare(logText.selectedText, "日本語");
                compare(logText.selectionStart, start);
            }
            mouseClick(logText, 30, 10, Qt.RightButton);
            compare(selectionMenu.selectedText, "日本語");
            logText.deselect();
            tryCompare(scroll, "appliedText", backend.highlightedText);
            verify(logText.getText(0, logText.length).includes("追加2"));
            backend.highlightedText = before;
            wait(50);
            follow.checked = true;
        }

        function test_drag_survives_stream_updates() {
            const before = backend.highlightedText;
            follow.checked = false;
            scroll.contentItem.contentY = 0;
            backend.highlightedText = "<pre>日本語 ERROR 障害\n次の行</pre>";
            wait(50);
            const visible = logText.text;
            const start = logText.positionToRectangle(4);
            const end = logText.positionToRectangle(9);
            mousePress(logText, start.x + 1, start.y + start.height / 2);
            backend.highlightedText = "<pre>日本語 ERROR 障害\n次の行\n追加</pre>";
            wait(30);
            compare(logText.text, visible, "Do not replace the document during a press");
            mouseMove(logText, end.x + 1, end.y + end.height / 2);
            mouseRelease(logText, end.x + 1, end.y + end.height / 2);
            verify(logText.selectedText.length > 0);
            verify(logText.selectionStart > 0, "The drag anchor must not jump to the beginning");
            compare(logText.text, visible);
            logText.deselect();
            tryCompare(scroll, "appliedText", backend.highlightedText);
            backend.highlightedText = before;
            wait(50);
            follow.checked = true;
        }
        function test_auto_scroll_resumes_selected_view() {
            const before = backend.highlightedText;
            follow.checked = false;
            logText.select(7, 13);
            const selected = logText.selectedText;
            verify(selected.length > 0);
            backend.highlightedText += "<br>Received during selection";
            wait(30);
            compare(logText.selectedText, selected);
            follow.checked = true;
            tryCompare(scroll, "appliedText", backend.highlightedText);
            compare(logText.selectedText, "");
            backend.highlightedText = before;
            wait(50);
        }

        function test_status_update_does_not_reset_cursor() {
            follow.checked = false;
            logText.deselect();
            logText.cursorPosition = 10;
            backend.changed();
            wait(30);
            compare(logText.cursorPosition, 10);
            follow.checked = true;
        }
        function test_right_click_preserves_selection() {
            follow.checked = false;
            scroll.contentItem.contentY = 0;
            logText.select(0, 6);
            compare(logText.selectedText, "Line 0");
            mouseClick(logText, 30, 10, Qt.RightButton);
            compare(selectionMenu.selectedText, "Line 0");
            compare(logText.selectedText, "Line 0");
            logText.deselect();
            follow.checked = true;
        }

        function test_auto_scroll_toggle() {
            follow.checked = true;
            compare(follow.checkable, true);
            compare(follow.background.color.toString(), "#0078d4");
            compare(follow.contentItem.color.toString(), "#ffffff");
            mouseClick(follow);
            compare(follow.checked, false);
            compare(follow.background.color.toString(), "#300078d4");
            compare(follow.contentItem.color.toString(), "#ffffff");
            scroll.contentItem.contentY = 0;
            mouseClick(follow);
            compare(follow.checked, true);
            compare(follow.background.color.toString(), "#0078d4");
            tryVerify(() => Math.abs(scroll.contentItem.contentY
                - (scroll.contentItem.contentHeight - scroll.contentItem.height)) < 2);
        }

        function test_selected_background_is_one_line() {
            const band = findChild(logText, "selectedLineBackground");
            verify(band !== null);
            backend.selectedPosition = 7;
            wait(100);
            verify(band.visible);
            const selected = logText.positionToRectangle(7);
            const next = logText.positionToRectangle(14);
            compare(band.y, selected.y);
            compare(band.height, selected.height);
            verify(band.y + band.height <= next.y + 1);
            verify(band.opacity < 1);
            backend.selectedPosition = -1;
            compare(band.visible, false);
        }

        function test_no_bounce_at_bounds() {
            wait(200);
            const flick = scroll.contentItem;
            compare(flick.boundsBehavior, Flickable.StopAtBounds);
            follow.checked = false;
            flick.contentY = 0;
            for (let i = 0; i < 10; ++i) {
                mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, 120);
                wait(20);
                compare(flick.contentY, 0, "Must not scroll beyond the top");
                compare(flick.verticalOvershoot, 0);
            }
            logText.followEnd();
            flick.contentY = flick.contentHeight - flick.height;
            const bottom = flick.contentY;
            for (let i = 0; i < 10; ++i) {
                mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, -120);
                wait(20);
                compare(flick.contentY, bottom, "Must not scroll beyond the bottom");
                compare(flick.verticalOvershoot, 0);
            }
            follow.checked = true;
        }

        function test_wheel_and_follow() {
            wait(200);
            logText.followEnd();
            wait(100);
            const bottom = scroll.contentItem.contentY;
            verify(bottom > 0, "Fixture must overflow the viewport");

            mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, 120);
            tryCompare(follow, "checked", false);
            compare(follow.background.color.toString(), "#300078d4");
            tryVerify(() => scroll.contentItem.contentY < bottom);

            const aboveBottom = scroll.contentItem.contentY;
            mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, -120);
            tryVerify(() => scroll.contentItem.contentY > aboveBottom);
            compare(follow.checked, false, "Scrolling down must not re-enable follow");

            mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, 120);
            tryVerify(() => scroll.contentItem.contentY < bottom);
            const paused = scroll.contentItem.contentY;
            backend.highlightedText += "<br>Appended while paused";
            wait(100);
            compare(scroll.contentItem.contentY, paused, "New logs must preserve the paused position");
            compare(follow.checked, false);

            follow.checked = true;
            logText.followEnd();
            verify(scroll.contentItem.contentY > paused);
            backend.highlightedText += "<br>Appended while following";
            tryVerify(() => Math.abs(scroll.contentItem.contentY
                - (scroll.contentItem.contentHeight - scroll.contentItem.height)) < 2);
            follow.checked = false;
            const pausedBottom = scroll.contentItem.contentY;
            backend.highlightedText += "<br>Appended at bottom without follow";
            wait(100);
            compare(scroll.contentItem.contentY, pausedBottom);
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-scroll-test-") as directory:
    path = Path(directory) / "tst_LogScroll.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)