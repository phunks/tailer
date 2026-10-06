#!/usr/bin/env python3
"""Pixel regression for Qt Quick's translucent <pre> newline background bleed.

The fixed HTML structure is also asserted by the Rust trap unit tests.
Run with Qt's qmltestrunner on PATH.
"""
import os
from pathlib import Path
import subprocess
import tempfile

fixture = r'''
import QtQuick
import QtQuick.Controls.Fusion
import QtTest
Item {
    width: 600
    height: 240
    TextArea {
        id: log
        width: 580
        height: 220
        font.family: "monospace"
        textFormat: TextEdit.RichText
        wrapMode: TextEdit.NoWrap
        color: "black"
        background: Rectangle { id: base; color: "white" }
    }
    TestCase {
        name: "BackgroundHighlightPixels"
        when: windowShown
        function test_rows_data() {
            return [
                {tag:"light", background:"#ffffff", foreground:"#000000", tinted:"#ffbfbf"},
                {tag:"dark", background:"#1b1b1b", foreground:"#eeeeee", tinted:"#541414"}
            ];
        }
        function test_rows(data) {
            base.color = data.background;
            log.color = data.foreground;
            // Two adjacent highlighted rows, an unhighlighted row, and an
            // isolated highlighted row must all have exactly one background coat.
            const plain = "xxxxxxxxxxxxxxxxxxxxxxxx";
            const highlighted = '<span style="background-color:#40ff0000">' + plain + '</span>';
            log.text = '<pre>' + [highlighted, highlighted, plain, highlighted, ""].join('<br/>') + '</pre>';
            wait(100);
            const image = grabImage(log);
            for (let line = 0; line < 4; ++line) {
                const rect = log.positionToRectangle(line * 25);
                // Top of the character box is background-only (above glyph ink).
                const color = image.pixel(Math.ceil(rect.x + 2), Math.ceil(rect.y + 1));
                compare(color.toString(), line === 2 ? data.background : data.tinted,
                        "Background must not bleed or become darker on row " + line);
            }
            // QTextDocument represents <br/> as Unicode line separators internally.
            compare(log.getText(0, log.length).replace(/\u2028/g, '\n'), [plain, plain, plain, plain, ""].join('\n'));
            const first = log.positionToRectangle(0);
            const second = log.positionToRectangle(25);
            compare(second.y - first.y, first.height);
        }
    }
}
'''
with tempfile.TemporaryDirectory(prefix="tailer-background-") as directory:
    path = Path(directory) / "tst_BackgroundHighlight.qml"
    path.write_text(fixture)
    result = subprocess.run(["qmltestrunner", "-input", str(path)],
                            env={**os.environ, "QT_QPA_PLATFORM": "offscreen"}, timeout=30)
    raise SystemExit(result.returncode)