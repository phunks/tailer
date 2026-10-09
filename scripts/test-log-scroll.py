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
    readonly property var logText: logDocument.item
    function tr(key) { return key; }
    QtObject { id: page; readonly property var logText: logDocument.item; property bool follow: true; property bool viewActive: true; function saveView() {} }
    QtObject { id: selectionMenu; property string selectedText: ""; property var textItem: null; function popup() {} }
    QtObject {
        id: backend
        property int selectedPosition: -1
        property int viewRevision: 0
        property string text: ""
        property var lineIds: []
        signal changed()
        onHighlightedTextChanged: {
            text = highlightedText.replace(/<br\s*\/?\s*>/g, "\n").replace(/<[^>]*>/g, "");
            lineIds = text === "" ? [] : text.split("\n").map((line, i) => {
                const number = line.match(/^(?:Line|Rolled) (\d+)/);
                return number ? Number(number[1]) + 1 : i + 1;
            });
            changed();
        }
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
        function init() {
            page.viewActive = true;
            backend.selectedPosition = -1;
            follow.checked = true;
            logText.deselect();
            backend.highlightedText = Array.from({length: 300}, (_, i) => "Line " + i).join("<br>");
            wait(50);
        }
        function cleanup() { follow.checked = true; logText.deselect(); wait(50); }
        function applyExplicitText(text) {
            backend.viewRevision++;
            backend.highlightedText = text;
            backend.changed();
        }
        function test_hidden_tab_defers_document_updates() {
            const text = logText.text;
            const serial = scroll.textUpdateSerial;
            page.viewActive = false;
            for (let i = 0; i < 5; ++i) {
                backend.highlightedText += "<br>Hidden append " + i;
                wait(20);
            }
            compare(logText.text, text);
            compare(scroll.textUpdateSerial, serial);
            page.viewActive = true;
            backend.changed();
            tryVerify(() => logText.text.includes("Hidden append 4"));
            compare(scroll.textUpdateSerial, serial + 1);
        }
        function test_paused_empty_view_keeps_appending_after_filling() {
            applyExplicitText("");
            wait(50);
            follow.checked = false;
            let lines = [];
            for (let i = 0; i < 60; ++i) {
                lines.push("Gradual " + i);
                backend.highlightedText = lines.join("<br>");
                wait(20);
                compare(scroll.appliedText, backend.highlightedText);
                compare(scroll.contentItem.contentY, 0);
            }
            verify(!scroll.displayFrozen);
            verify(scroll.contentItem.contentHeight > scroll.contentItem.height);
            backend.highlightedText += "<br>Still appending";
            wait(50);
            compare(scroll.appliedText, backend.highlightedText);
            root.height += 100;
            backend.changed();
            wait(50);
            compare(scroll.appliedText, backend.highlightedText);
            root.height -= 100;
            follow.checked = true;
            tryCompare(scroll, "appliedText", backend.highlightedText);
        }
        function test_paused_full_view_tracks_line_during_ring_eviction() {
            follow.checked = false;
            scroll.contentItem.contentY = 1200;
            const y = scroll.contentItem.contentY;
            const offsets = scroll.lineOffsets(backend.text);
            let anchor = 0;
            while (logText.positionToRectangle(offsets[anchor]).y + logText.positionToRectangle(offsets[anchor]).height <= y) ++anchor;
            const screenY = logText.positionToRectangle(offsets[anchor]).y - y;
            for (let i = 0; i < 10; ++i) {
                backend.highlightedText = Array.from({length: 300}, (_, n) => "Rolled " + (n + i)).join("<br>");
                wait(20);
                compare(scroll.appliedText, backend.highlightedText);
                const nextOffsets = scroll.lineOffsets(backend.text);
                const index = backend.lineIds.indexOf(anchor + 1);
                verify(index >= 0);
                compare(logText.positionToRectangle(nextOffsets[index]).y - scroll.contentItem.contentY, screenY);
                verify(!scroll.displayFrozen);
            }
            verify(scroll.contentItem.contentY < y);
            const frozen = logText.text;
            backend.highlightedText = Array.from({length: 300}, (_, n) => "Rolled " + (n + 300)).join("<br>");
            wait(50);
            compare(logText.text, frozen);
            verify(scroll.displayFrozen);
            mouseWheel(scroll, scroll.width / 2, scroll.height / 2, 0, 120);
            tryCompare(scroll, "appliedText", backend.highlightedText);
            compare(scroll.contentItem.contentY, 0);
            compare(scroll.displayFrozen, false);
            applyExplicitText("Filtered match");
            wait(50);
            compare(scroll.appliedText, "Filtered match");
            compare(scroll.displayFrozen, false);
            backend.highlightedText = "Filtered match<br>Second match";
            wait(50);
            compare(scroll.appliedText, backend.highlightedText);
            applyExplicitText(Array.from({length: 300}, (_, n) => "Context " + n).join("<br>"));
            wait(50);
            backend.highlightedText += "<br>Incoming context update";
            wait(50);
            compare(scroll.appliedText, backend.highlightedText);
        }
        function test_paused_top_freezes_only_when_read_line_is_evicted() {
            follow.checked = false;
            scroll.contentItem.contentY = 0;
            const before = logText.text;
            backend.highlightedText += "<br>Appended at top";
            wait(50);
            compare(scroll.appliedText, backend.highlightedText);
            compare(scroll.contentItem.contentY, 0);
            const frozen = logText.text;
            backend.highlightedText = Array.from({length: 300}, (_, n) => "Rolled " + (n + 1)).join("<br>");
            wait(50);
            verify(scroll.displayFrozen);
            compare(logText.text, frozen);
            // Keyboard handling requires the log document to have focus.
            logText.forceActiveFocus();
            keyClick(Qt.Key_Down);
            tryCompare(scroll, "appliedText", backend.highlightedText);
            compare(scroll.contentItem.contentY, 0);
        }
        function test_identical_lines_still_track_source_ids() {
            applyExplicitText(Array.from({length: 300}, () => "same log").join("<br>"));
            wait(50);
            follow.checked = false;
            scroll.contentItem.contentY = 1200;
            const y = scroll.contentItem.contentY;
            backend.lineIds = backend.lineIds.map(id => id + 1);
            backend.changed();
            wait(50);
            verify(scroll.contentItem.contentY < y);
            compare(scroll.displayFrozen, false);
            scroll.contentItem.contentY = 0;
            backend.lineIds = backend.lineIds.map(id => id + 1);
            backend.changed();
            wait(50);
            verify(scroll.displayFrozen, "Identical text must not hide eviction of the viewed source line");
        }
        function test_paused_view_does_not_jump_during_updates() {
            const before = backend.highlightedText;
            follow.checked = false;
            const lines = Array.from({length: 300}, (_, i) => "Line " + i + " " + "wide log ".repeat(40));
            applyExplicitText("<pre>" + lines.join("<br>") + "</pre>");
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
                    compare(scroll.appliedText, backend.highlightedText);
                }
                compare(shiftedPausedFrames, 0, "Every frame must keep the paused viewport stable");
            } finally {
                watchingPaused = false;
                backend.highlightedText = before;
                wait(50);
                follow.checked = true;
            }
        }
        function test_context_navigation_starts_at_left_and_shows_target() {
            follow.checked = false;
            applyExplicitText("<pre>" + Array.from({length: 100}, (_, i) =>
                "Line " + i + " " + "wide log ".repeat(200)).join("<br>") + "</pre>");
            wait(50);
            scroll.contentItem.contentX = 4000;
            scroll.contentItem.contentY = 1000;
            verify(scroll.contentItem.contentX > 1000);
            const context = Array.from({length: 21}, (_, i) =>
                (58 + i) + ": " + (i === 10 ? "対象行 あ" : "context " + "wide log ".repeat(200)));
            const plain = context.join("\n");
            backend.selectedPosition = plain.indexOf("68: ");
            applyExplicitText("<pre>" + context.join("<br>") + "</pre>");
            wait(100);
            compare(scroll.contentItem.contentX, 0, "Context navigation must not restore the old right-hand offset");
            const rect = logText.positionToRectangle(backend.selectedPosition);
            verify(rect.y >= scroll.contentItem.contentY);
            verify(rect.y + rect.height <= scroll.contentItem.contentY + scroll.contentItem.height);
            verify(logText.getText(backend.selectedPosition, backend.selectedPosition + 4).startsWith("68:"));
            scroll.contentItem.contentX = 1000;
            backend.viewRevision++;
            backend.changed();
            wait(50);
            compare(scroll.contentItem.contentX, 0, "Clicking the same result must also reset its horizontal position");
            // A second navigation before deferred layout settles must win.
            backend.selectedPosition = 0;
            applyExplicitText("<pre>1: short あ</pre>");
            backend.selectedPosition = 0;
            applyExplicitText("<pre>2: short あ</pre>");
            wait(100);
            compare(scroll.contentItem.contentX, 0);
            compare(scroll.contentItem.contentY, 0);
        }
        function test_context_from_auto_scroll_renders_glyphs_not_just_highlight() {
            logText.color = "white";
            logText.palette.base = "#1b1b1b";
            follow.checked = true;
            applyExplicitText("<pre>" + Array.from({length: 2000}, (_, i) =>
                "Line " + i + ' 日本語 <span style="color:#35c66b;font-weight:bold">あ</span>').join("\\n") + "</pre>");
            wait(100);
            verify(scroll.contentItem.contentY > 10000);
            // Reproduce the reported history using actual button presses:
            // once enabled, turning it off must not poison later context rendering.
            mouseClick(follow);
            compare(follow.checked, false);
            mouseClick(follow);
            compare(follow.checked, true);
            wait(50);
            mouseClick(follow);
            compare(follow.checked, false);
            const liveDocument = logText;
            follow.checked = false;
            // Match the real backend: all view properties change before one notification.
            backend.viewRevision++;
            backend.selectedPosition = 0;
            backend.highlightedText = "<pre>" + Array.from({length: 11}, (_, i) =>
                (i + 1) + ': TEST 日本語 <span style="color:#35c66b;font-weight:bold">あ</span>').join("\\n") + "\\n</pre>";
            backend.changed();
            wait(100);
            verify(logText !== liveDocument, "Context navigation must create a fresh rendering item");
            logText.color = "white";
            logText.palette.base = "#1b1b1b";
            wait(50);
            compare(scroll.contentItem.contentX, 0);
            compare(scroll.contentItem.contentY, 0);
            verify(logText.length > 0);
            const band = findChild(logText, "selectedLineBackground");
            verify(band.visible);
            verify(logText.width >= scroll.availableWidth - 1, "Short context must fill the pane width");
            verify(logText.height >= scroll.availableHeight - 1, "Short context must fill the pane height");
            const image = grabImage(scroll);
            const rect = logText.positionToRectangle(0);
            const point = logText.mapToItem(scroll, rect.x, rect.y);
            let glyphs = 0;
            for (let y = Math.ceil(point.y); y < Math.floor(point.y + rect.height); ++y)
                for (let x = Math.ceil(point.x); x < Math.ceil(point.x) + 180; ++x) {
                    const pixel = image.pixel(x, y);
                    if (pixel.r > 0.8 && pixel.g > 0.8 && pixel.b > 0.8) ++glyphs;
                }
            verify(glyphs > 10, "Target row must paint actual text, not only the blue highlight; pixels=" + glyphs);
            for (let step = 0; step < 8; ++step) {
                backend.selectedPosition = -1;
                follow.checked = true;
                applyExplicitText("<pre>" + Array.from({length: 2000}, (_, i) =>
                    "Line " + i + " TEST 日本語 あ").join("\\n") + "</pre>");
                wait(20);
                follow.checked = false;
                backend.viewRevision++;
                backend.selectedPosition = 0;
                backend.highlightedText = "<pre>1: TEST 日本語 あ\\n2: 次の行\\n</pre>";
                backend.changed();
                wait(40);
                logText.color = "white";
                logText.palette.base = "#1b1b1b";
                wait(20);
                compare(scroll.contentItem.contentX, 0);
                compare(scroll.contentItem.contentY, 0);
                const frame = grabImage(scroll);
                const top = logText.positionToRectangle(0);
                const origin = logText.mapToItem(scroll, top.x, top.y);
                let ink = 0;
                for (let y = Math.ceil(origin.y); y < Math.floor(origin.y + top.height); ++y)
                    for (let x = Math.ceil(origin.x); x < Math.ceil(origin.x) + 180; ++x) {
                        const color = frame.pixel(x, y);
                        if (color.r > 0.8 && color.g > 0.8 && color.b > 0.8) ++ink;
                    }
                verify(ink > 10, "Context text must render after repeated live/context transitions; step=" + step);
            }
        }
        function test_short_document_fills_pane_after_resize() {
            follow.checked = false;
            backend.selectedPosition = 0;
            applyExplicitText("<pre>1: short あ</pre>");
            wait(50);
            for (let i = 0; i < 2; ++i) {
                root.width = i === 0 ? 1000 : 800;
                root.height = i === 0 ? 700 : 600;
                wait(50);
                verify(logText.width >= scroll.availableWidth - 1);
                verify(logText.height >= scroll.availableHeight - 1);
                const band = findChild(logText, "selectedLineBackground");
                compare(band.width, logText.width - logText.leftPadding - logText.rightPadding);
                verify(scroll.ScrollBar.vertical.visible);
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
            applyExplicitText("<pre>先頭\n日本語 ERROR 障害\n末尾</pre>");
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
            applyExplicitText("<pre>日本語 ERROR 障害\n次の行</pre>");
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
        function test_selected_background_follows_document_when_scrolling() {
            follow.checked = false;
            scroll.contentItem.contentY = 0;
            const band = findChild(logText, "selectedLineBackground");
            const position = backend.text.indexOf("Line 10\n");
            verify(position >= 0);
            backend.selectedPosition = position;
            wait(50);
            function verifyAlignment() {
                const rect = logText.positionToRectangle(position);
                const line = logText.mapToItem(root, 0, rect.y);
                const highlight = band.mapToItem(root, 0, 0);
                verify(Math.abs(highlight.y - line.y) < 1, "Highlight must follow the document, not the viewport");
                compare(band.height, rect.height);
            }
            verifyAlignment();
            const initialY = band.mapToItem(root, 0, 0).y;
            scroll.contentItem.contentY = 60;
            wait(50);
            verifyAlignment();
            compare(band.mapToItem(root, 0, 0).y, initialY - 60);
            scroll.contentItem.contentY = 300;
            wait(50);
            verifyAlignment();
            scroll.contentItem.contentY = 0;
            wait(50);
            verifyAlignment();
            backend.selectedPosition = -1;
            compare(band.visible, false);
        }

        function test_no_bounce_at_bounds() {
            compare(scroll.ScrollBar.vertical.policy, ScrollBar.AlwaysOn);
            verify(scroll.ScrollBar.vertical.visible);
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
            wait(500); // Let ScrollView's wheel animation settle before recording the position.
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
        env={**os.environ, "QT_QPA_PLATFORM": os.environ.get("TAILER_TEST_QPA_PLATFORM", "offscreen")},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)