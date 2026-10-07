#!/usr/bin/env python3
"""Exercise production filter controls and trap right-click enable actions."""

import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parent.parent
source = (root / "src/Main.qml").read_text()
start = source.index("                        RowLayout {\n                            id: filterRow")
end = source.index("                        RowLayout {\n                            id: searchRow", start)
filter_controls = source[start:end]
start = source.index("                    function applyFilter()")
end = source.index("                    required property string searchText", start)
apply_filter = source[start:end]
start = source.index("                    function setTrapEnabled(")
end = source.index("                    function editTrap(", start)
set_trap_enabled = source[start:end]
start = source.index("                                delegate: Row {\n                                    id: trapTag")
end = source.index("                            }\n                        }\n                        Label {", start)
trap_delegate = source[start:end]

fixture = """
import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtTest
Item {
    id: root
    width: 1100; height: 400
    function tr(key) { return key; }
    QtObject { id: optionsToggle; property bool checked: true }
    QtObject {
        id: backend
        property var applied: []
        function set_numeric_filter(text, regex, ignoreCase, invert, numeric) {
            applied = [text, regex, ignoreCase, invert, numeric];
        }
    }
    QtObject {
        id: page
        property string filterText: "ERROR (\\\\d+)"
        property bool filterEnabled: true
        property bool filterRegex: true
        property bool filterCase: true
        property bool filterInvert: true
        property var filterNumeric: ({capture:1, operator:">", value:"100"})
        property var trapRules: [{text:"ERROR.*", regex:true, color:"#35c66b"}]
        property int editIndex: -1
        function numericFilterCondition() {
            return filterNumericEnabled.checked
                ? {capture:filterCapture.value, operator:filterOperator.currentText, value:filterValue.text} : null;
        }
        function saveView() { filterEnabled = filterEnable.checked; }
        function commitTrapRules(rules) { trapRules = rules; return true; }
        function editTrap(index) { editIndex = index; }
        function removeTrap(index) {}
""" + apply_filter + set_trap_enabled + """
    }
    ColumnLayout {
        anchors.fill: parent
""" + filter_controls + """
        Repeater {
            id: tags
            model: page.trapRules
""" + trap_delegate + """
        }
        Item { Layout.fillHeight: true }
    }
    TestCase {
        name: "RuleEnable"
        when: windowShown
        function test_filter_keeps_expression_and_numeric_options() {
            const expression = filterInput.text;
            page.applyFilter();
            compare(backend.applied[0], expression);
            compare(backend.applied[4].value, "100");
            mouseClick(filterEnable);
            compare(filterEnable.checked, false);
            compare(page.filterEnabled, false);
            compare(backend.applied[0], "");
            compare(backend.applied[3], false, "Invert must not hide all logs when disabled");
            compare(backend.applied[4], null);
            compare(filterInput.text, expression);
            verify(filterRegex.checked && filterCase.checked && filterInvert.checked);
            verify(filterNumericEnabled.checked);
            filterTimer.restart();
            wait(350);
            compare(backend.applied[0], "", "Editing must not re-enable the filter");
            mouseClick(filterEnable);
            compare(backend.applied[0], expression);
            compare(backend.applied[3], true);
            compare(backend.applied[4].value, "100");
        }
        function test_trap_right_click_toggle() {
            for (let i = 0; i < 4; ++i) {
                const row = tags.itemAt(0);
                const button = row.children[0];
                const menu = findChild(row, "trapTagMenu");
                mouseClick(button, button.width / 2, button.height / 2, Qt.RightButton);
                tryCompare(menu, "opened", true);
                compare(page.editIndex, -1, "Right-click must not open the editor");
                const action = menu.itemAt(0);
                compare(action.checked, i % 2 === 0);
                mouseClick(action);
                wait(100);
                compare(page.trapRules[0].enabled, i % 2 !== 0);
                compare(page.trapRules[0].text, "ERROR.*");
                compare(tags.itemAt(0).children[0].opacity, i % 2 === 0 ? 0.45 : 1);
            }
        }
    }
}
"""

with tempfile.TemporaryDirectory(prefix="tailer-rule-enable-") as directory:
    path = Path(directory) / "tst_RuleEnable.qml"
    path.write_text(fixture)
    result = subprocess.run(
        ["qmltestrunner", "-input", str(path)],
        env={**os.environ, "QT_QPA_PLATFORM": "offscreen"},
        timeout=30,
        check=False,
    )
    raise SystemExit(result.returncode)