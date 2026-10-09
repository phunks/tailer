pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls.Fusion
import QtQuick.Layouts
import QtQuick.Dialogs
import tailer

ApplicationWindow {
    id: root
    width: 1100
    height: 720
    visible: true
    title: "Tailer"
    property string smokePath: ""
    property string analysisTestPath: ""
    property bool analysisPresetRestoreTest: false
    property bool restoreTest: false
    property bool connectionUiTest: false
    property string i18nTestLanguage: ""
    property bool localCommandTest: false
    property string syslogTestEndpoint: ""
    property string bookmarkTestMode: ""
    property string themeTestMode: ""
    property bool tabUiTest: false
    property bool movingTab: false
    readonly property bool darkAppearance: settings.appearance === "dark"
        || (settings.appearance === "auto" && Qt.styleHints.colorScheme === Qt.Dark)
    // Explicit palettes also support platforms that cannot override the system scheme.
    palette.window: darkAppearance ? "#252525" : "#efefef"
    palette.windowText: darkAppearance ? "#eeeeee" : "#202020"
    palette.base: darkAppearance ? "#1b1b1b" : "#ffffff"
    palette.alternateBase: darkAppearance ? "#303030" : "#f7f7f7"
    palette.text: darkAppearance ? "#eeeeee" : "#202020"
    palette.button: darkAppearance ? "#353535" : "#e5e5e5"
    palette.buttonText: darkAppearance ? "#eeeeee" : "#202020"
    palette.highlight: darkAppearance ? "#3d8edb" : "#0078d4"
    palette.highlightedText: "#ffffff"
    palette.toolTipBase: darkAppearance ? "#353535" : "#ffffdc"
    palette.toolTipText: darkAppearance ? "#eeeeee" : "#202020"
    palette.placeholderText: darkAppearance ? "#aaaaaa" : "#707070"
    palette.light: darkAppearance ? "#555555" : "#ffffff"
    palette.midlight: darkAppearance ? "#454545" : "#f4f4f4"
    palette.mid: darkAppearance ? "#606060" : "#b8b8b8"
    palette.dark: darkAppearance ? "#181818" : "#808080"
    palette.shadow: darkAppearance ? "#101010" : "#505050"
    palette.link: darkAppearance ? "#70b7ff" : "#0066cc"
    palette.linkVisited: darkAppearance ? "#c49bff" : "#8040a0"
    palette.brightText: "#ffffff"
    palette.accent: darkAppearance ? "#3d8edb" : "#0078d4"
    palette.disabled.text: darkAppearance ? "#808080" : "#909090"
    palette.disabled.windowText: darkAppearance ? "#808080" : "#909090"
    palette.disabled.buttonText: darkAppearance ? "#808080" : "#909090"
    function applyAppearance() {
        Qt.styleHints.colorScheme = settings.appearance === "dark" ? Qt.Dark
            : settings.appearance === "light" ? Qt.Light : Qt.Unknown;
    }
    Translations {
        id: translations
        onChanged: {
            workspace.refresh_language();
            for (let i = 0; i < logs.count; ++i) {
                // qmllint disable missing-property
                const page = pageRepeater.itemAt(i);
                if (page) page.backend.refresh_language();
                // qmllint enable missing-property
            }
            root.scheduleSave();
        }
    }
    function tr(key) { return translations.messages[key] || key; }
    function translateButtons(dialog) {
        const labels = [[Dialog.Ok, "OK"], [Dialog.Cancel, "Cancel"], [Dialog.Save, "Save"], [Dialog.Close, "Close"]];
        for (const pair of labels) {
            const button = dialog.standardButton(pair[0]);
            if (button) button.text = Qt.binding(() => root.tr(pair[1]));
        }
    }
    property int smokeStage: 0

    Workspace {
        id: workspace
    }
    ConnectionManager {
        id: connectionManager
        onChanged: {
            if (authPrompt !== "" && !connectionAuth.visible)
                connectionAuth.open();
        }
    }
    Timer {
        interval: 250
        repeat: true
        running: true
        onTriggered: connectionManager.poll()
    }
    ListModel { id: connectionProfiles }
    ListModel { id: bookmarks }
    ListModel { id: bookmarkGroups }
    ListModel { id: bookmarkTree }
    property bool unclassifiedExpanded: true
    function groupIndex(id) {
        for (let i = 0; i < bookmarkGroups.count; ++i)
            if (bookmarkGroups.get(i).id === id) return i;
        return -1;
    }
    function rebuildBookmarkTree() {
        bookmarkTree.clear();
        function appendChildren(parentId, depth) {
            if (depth > 100) return;
            for (let i = 0; i < bookmarkGroups.count; ++i) {
                const group = bookmarkGroups.get(i);
                if (group.parentId !== parentId) continue;
                bookmarkTree.append({isGroup:true, nodeId:group.id, sourceIndex:i, depth:depth,
                    bookmarkTitle:group.name, logPath:"", source:"", remote:false, connectionId:"", expanded:group.expanded});
                if (group.expanded) appendChildren(group.id, depth + 1);
            }
            if (parentId !== "") appendBookmarks(parentId, depth);
        }
        function appendBookmarks(groupId, depth) {
            for (let i = 0; i < bookmarks.count; ++i) {
                const row = bookmarks.get(i);
                if ((row.groupId || "") !== groupId) continue;
                bookmarkTree.append({isGroup:false, nodeId:"", sourceIndex:i, depth:depth,
                    bookmarkTitle:row.bookmarkTitle, logPath:row.logPath, source:row.source || "file", remote:row.remote,
                    connectionId:row.connectionId, expanded:false});
            }
        }
        appendChildren("", 0);
        bookmarkTree.append({isGroup:true, nodeId:"", sourceIndex:-1, depth:0,
            bookmarkTitle:root.tr("Unclassified"), logPath:"", source:"", remote:false, connectionId:"", expanded:unclassifiedExpanded});
        if (unclassifiedExpanded) appendBookmarks("", 1);
    }
    function toggleBookmarkGroup(id) {
        const index = groupIndex(id);
        if (index >= 0) bookmarkGroups.setProperty(index, "expanded", !bookmarkGroups.get(index).expanded);
        else unclassifiedExpanded = !unclassifiedExpanded;
        rebuildBookmarkTree();
        scheduleSave();
    }
    function groupContains(ancestor, child) {
        if (ancestor === "") return child === "";
        for (let i = 0; i <= bookmarkGroups.count; ++i) {
            if (child === ancestor) return true;
            const index = groupIndex(child);
            if (index < 0) return false;
            child = bookmarkGroups.get(index).parentId;
        }
        return false;
    }
    function openBookmarkGroup(id, confirmed = false) {
        const indices = [];
        let hasCommand = false;
        for (let i = 0; i < bookmarks.count; ++i) {
            const row = bookmarks.get(i);
            if (!groupContains(id, row.groupId || "")) continue;
            indices.push(i);
            if (row.source === "custom") hasCommand = true;
        }
        if (indices.length === 0) return;
        if (hasCommand && !confirmed) {
            bookmarkGroupConfirm.groupId = id;
            bookmarkGroupConfirm.open();
            return;
        }
        for (const index of indices) openBookmark(index);
        bookmarksDialog.close();
    }
    function editBookmarkGroup(id, creating = false) {
        const index = groupIndex(id);
        groupEditor.groupId = id;
        groupEditor.creating = creating;
        groupNameInput.text = creating || index < 0 ? "" : bookmarkGroups.get(index).name;
        groupEditor.open();
    }
    function deleteBookmarkGroup(id) {
        if (groupIndex(id) < 0) return;
        for (let i = 0; i < bookmarks.count; ++i)
            if (groupContains(id, bookmarks.get(i).groupId || "")) bookmarks.setProperty(i, "groupId", "");
        const removed = [];
        for (let i = 0; i < bookmarkGroups.count; ++i)
            if (groupContains(id, bookmarkGroups.get(i).id)) removed.push(i);
        for (let i = removed.length - 1; i >= 0; --i) bookmarkGroups.remove(removed[i]);
        rebuildBookmarkTree();
        saveWorkspace();
    }
    function profileById(id) {
        for (let i = 0; i < connectionProfiles.count; ++i)
            if (connectionProfiles.get(i).id === id)
                return connectionProfiles.get(i);
        return {};
    }
    function profileUsed(id) {
        for (let i = 0; i < logs.count; ++i)
            if (logs.get(i).connectionId === id)
                return true;
        for (let i = 0; i < bookmarks.count; ++i)
            if (bookmarks.get(i).remote && bookmarks.get(i).connectionId === id)
                return true;
        return false;
    }
    property bool restoring: true
    property var encodingOptions: ["UTF-8", "CP932", "EUC-JP", "ISO-2022-JP", "UTF-16LE", "UTF-16BE", "GB18030", "BIG5", "CP949", "WINDOWS-1252", "ISO-8859-1"]
    QtObject {
        id: settings
        property int initialLines: 50
        property int capacityMiB: 512
        property string appearance: "auto"
        onAppearanceChanged: {
            root.applyAppearance();
            root.scheduleSave();
        }
    }
    function scheduleSave() {
        if (!restoring && smokePath === "")
            saveTimer.restart();
    }
    function logConfiguration(row) {
        return {
            logPath: row.logPath,
            logTitle: row.logTitle,
            remote: row.remote,
            source: row.source,
            encoding: row.encoding,
            connectionId: row.connectionId,
            elevation: row.elevation,
            runUser: row.runUser,
            initial: row.initial,
            capacity: row.capacity,
            filterText: row.filterText,
            filterEnabled: row.filterEnabled !== false,
            filterRegex: row.filterRegex,
            filterCase: row.filterCase,
            filterInvert: row.filterInvert,
            filterNumericJson: row.filterNumericJson || "null",
            searchText: row.searchText,
            searchRegex: row.searchRegex,
            searchCase: row.searchCase,
            follow: row.follow,
            analysisPresetId: row.analysisPresetId || "builtin-apache",
            analysisMergeFirstColumn: row.analysisMergeFirstColumn === true,
            syslogBadges: row.syslogBadges === true,
            trapRulesJson: row.trapRulesJson
        };
    }
    function saveWorkspace() {
        if (restoring || smokePath !== "")
            return;
        const saved = [];
        for (let i = 0; i < logs.count; ++i)
            saved.push(logConfiguration(logs.get(i)));
        const savedBookmarks = [];
        for (let i = 0; i < bookmarks.count; ++i) {
            const row = bookmarks.get(i);
            const configuration = logConfiguration(row);
            configuration.bookmarkTitle = row.bookmarkTitle;
            configuration.groupId = row.groupId || "";
            savedBookmarks.push(configuration);
        }
        const profiles = [];
        const groups = [];
        for (let i = 0; i < bookmarkGroups.count; ++i) {
            const g = bookmarkGroups.get(i);
            groups.push({id:g.id, name:g.name, parentId:g.parentId, expanded:g.expanded});
        }
        for (let i = 0; i < connectionProfiles.count; ++i) {
            const p = connectionProfiles.get(i);
            profiles.push({id: p.id, name: p.name, host: p.host, user: p.user, port: p.port, key: p.key});
        }
        workspace.save({
            version: 2,
            language: translations.language,
            appearance: settings.appearance,
            connections: profiles,
            tabs: saved,
            bookmarks: savedBookmarks,
            bookmarkGroups: groups,
            initialLines: settings.initialLines,
            capacityMiB: settings.capacityMiB,
            currentIndex: tabs.currentIndex,
            width: root.width,
            height: root.height
        });
    }
    Timer {
        id: saveTimer
        interval: 500
        onTriggered: root.saveWorkspace()
    }
    onClosing: root.saveWorkspace()
    onWidthChanged: scheduleSave()
    onHeightChanged: scheduleSave()

    function localFilePath(url, platform = Qt.platform.os) {
        const match = /^file:\/\/([^/]*)(\/.*)$/i.exec(url.toString());
        if (!match) return url.toString();
        const host = match[1];
        let path = decodeURIComponent(match[2]);
        if (host !== "" && host.toLowerCase() !== "localhost")
            return "//" + host + path;
        // File URLs have a leading slash before Windows drive letters.
        if (platform === "windows" && /^\/[A-Za-z]:\//.test(path))
            path = path.slice(1);
        return path;
    }

    function addLog(path, source = "file", title = "", encoding = localEncoding.currentText) {
        logs.append({
            logPath: path,
            logTitle: title || path.split("/").pop(),
            remote: false,
            unread: false,
            alert: false,
            trapText: "",
            trapCase: false,
            trapRulesJson: "[]",
            source: source,
            encoding: encoding,
            connectionId: "",
            elevation: 0,
            runUser: "root",
            initial: settings.initialLines,
            capacity: settings.capacityMiB,
            filterText: "",
            filterEnabled: true,
            filterRegex: false,
            filterCase: false,
            filterInvert: false,
            filterNumericJson: "null",
            searchText: "",
            searchRegex: false,
            searchCase: false,
            follow: true,
            analysisPresetId: "builtin-apache",
            analysisMergeFirstColumn: false,
            syslogBadges: false
        });
        tabs.currentIndex = logs.count - 1;
        scheduleSave();
    }

    function closeTab(index) {
        if (index < 0 || index >= logs.count) return;
        // qmllint disable missing-property
        const page = pageRepeater.itemAt(index);
        if (page)
            page.backend.stop();
        // qmllint enable missing-property
        const current = tabs.currentIndex;
        const next = logs.count === 1 ? -1 : (index < current ? current - 1 : Math.min(current, logs.count - 2));
        logs.remove(index);
        tabs.currentIndex = next;
        scheduleSave();
    }

    function moveTab(from, to) {
        if (from < 0 || to < 0 || from >= logs.count || to >= logs.count || from === to) return;
        const selected = tabs.currentIndex;
        movingTab = true;
        logs.move(from, to, 1);
        if (selected === from) tabs.currentIndex = to;
        else if (from < selected && to >= selected) tabs.currentIndex = selected - 1;
        else if (from > selected && to <= selected) tabs.currentIndex = selected + 1;
        else tabs.currentIndex = selected;
        movingTab = false;
        // Repeater moves existing delegates; reconnecting would lose the current archive.
        for (let i = 0; i < logs.count; ++i) {
            // qmllint disable missing-property
            const page = pageRepeater.itemAt(i);
            const tab = tabRepeater.itemAt(i);
            if (page && tab) tab.connected = page.backend.connected;
            // qmllint enable missing-property
        }
        scheduleSave();
    }

    function dropTab(from, sceneX, sceneY) {
        const point = tabs.mapFromItem(null, sceneX, sceneY);
        let target = from;
        for (let i = 0; i < logs.count; ++i) {
            const tab = tabRepeater.itemAt(i);
            const position = tab.mapToItem(tabs, 0, 0);
            if (point.x >= position.x && point.x <= position.x + tab.width) { target = i; break; }
            if (point.x > position.x + tab.width) target = i;
            if (i === 0 && point.x < position.x) { target = 0; break; }
        }
        moveTab(from, target);
    }

    function closeTabs(mode, index) {
        if (mode === "single") { closeTab(index); return; }
        for (let i = logs.count - 1; i >= 0; --i)
            if (mode === "all" || i !== index) closeTab(i);
    }

    function restartTab(index) {
        // qmllint disable missing-property
        const page = pageRepeater.itemAt(index);
        if (page) page.restartSource();
        // qmllint enable missing-property
        logs.setProperty(index, "unread", false);
        logs.setProperty(index, "alert", false);
        tabs.currentIndex = index;
        scheduleSave();
    }

    function registerBookmark(index) {
        if (index < 0 || index >= logs.count) return;
        const configuration = logConfiguration(logs.get(index));
        // Registering an unchanged configuration twice does not create duplicates.
        const serialized = JSON.stringify(configuration);
        for (let i = 0; i < bookmarks.count; ++i) {
            if (JSON.stringify(logConfiguration(bookmarks.get(i))) === serialized) {
                bookmarkRegisteredDialog.message = root.tr("This configuration is already bookmarked.");
                bookmarkRegisteredDialog.open();
                return;
            }
        }
        if (bookmarks.count >= 100) return;
        configuration.bookmarkTitle = configuration.logTitle || configuration.logPath;
        configuration.groupId = "";
        bookmarks.append(configuration);
        rebuildBookmarkTree();
        saveWorkspace();
        bookmarkRegisteredDialog.message = workspace.error || root.tr("Bookmark registered.");
        bookmarkRegisteredDialog.open();
    }

    function openBookmark(index) {
        if (index < 0 || index >= bookmarks.count) return;
        const configuration = logConfiguration(bookmarks.get(index));
        if (configuration.remote && !profileById(configuration.connectionId).id) return;
        configuration.logTitle = bookmarks.get(index).bookmarkTitle;
        configuration.unread = false;
        configuration.alert = false;
        configuration.trapText = "";
        configuration.trapCase = false;
        logs.append(configuration);
        tabs.currentIndex = logs.count - 1;
        scheduleSave();
        bookmarksDialog.close();
    }

    function editBookmark(index) {
        if (index < 0 || index >= bookmarks.count) return;
        bookmarkEditor.bookmarkIndex = index;
        bookmarkTitleInput.text = bookmarks.get(index).bookmarkTitle;
        bookmarkGroupSelection.currentIndex = Math.max(0, bookmarkGroupSelection.ids.indexOf(bookmarks.get(index).groupId || ""));
        bookmarkEditor.open();
    }

    function deleteBookmark(index) {
        if (index < 0 || index >= bookmarks.count) return;
        bookmarks.remove(index);
        rebuildBookmarkTree();
        saveWorkspace();
    }

    Dialog {
        id: bookmarkRegisteredDialog
        property string message: ""
        width: Math.min(420, root.width - 40)
        title: root.tr("Bookmarks")
        anchors.centerIn: parent
        modal: true
        standardButtons: Dialog.Ok
        onOpened: root.translateButtons(bookmarkRegisteredDialog)
        Label { width: parent.width; text: bookmarkRegisteredDialog.message; wrapMode: Text.WordWrap }
    }

    Dialog {
        id: bookmarkEditor
        property int bookmarkIndex: -1
        title: root.tr("Edit bookmark")
        anchors.centerIn: parent
        modal: true
        width: Math.min(480, root.width - 40)
        standardButtons: Dialog.Save | Dialog.Cancel
        onOpened: {
            root.translateButtons(bookmarkEditor);
            standardButton(Dialog.Save).enabled = Qt.binding(() => bookmarkTitleInput.text.trim() !== "");
            bookmarkTitleInput.forceActiveFocus();
            bookmarkTitleInput.selectAll();
        }
        onAccepted: {
            if (bookmarkIndex < 0 || bookmarkIndex >= bookmarks.count || bookmarkTitleInput.text.trim() === "") return;
            bookmarks.setProperty(bookmarkIndex, "bookmarkTitle", bookmarkTitleInput.text.trim());
            bookmarks.setProperty(bookmarkIndex, "groupId", bookmarkGroupSelection.ids[bookmarkGroupSelection.currentIndex]);
            root.rebuildBookmarkTree();
            root.saveWorkspace();
        }
        ColumnLayout {
            anchors.fill: parent
            Label { text: root.tr("Bookmark title (required)") }
            TextField {
                id: bookmarkTitleInput
                maximumLength: 8192
                Layout.fillWidth: true
                onAccepted: if (text.trim() !== "") bookmarkEditor.accept()
            }
            Label {
                text: root.tr("Changes affect this bookmark only, not existing tabs.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label { text: root.tr("Group") }
            ComboBox {
                id: bookmarkGroupSelection
                property var ids: {
                    const values = [""];
                    for (let i = 0; i < bookmarkGroups.count; ++i) values.push(bookmarkGroups.get(i).id);
                    return values;
                }
                model: {
                    const values = [root.tr("Unclassified")];
                    for (let i = 0; i < bookmarkGroups.count; ++i) {
                        const group = bookmarkGroups.get(i);
                        let name = group.name;
                        let parent = group.parentId;
                        for (let depth = 0; parent !== "" && depth < 100; ++depth) {
                            const index = root.groupIndex(parent);
                            if (index < 0) break;
                            name = bookmarkGroups.get(index).name + " / " + name;
                            parent = bookmarkGroups.get(index).parentId;
                        }
                        values.push(name);
                    }
                    return values;
                }
                Layout.fillWidth: true
            }
        }
    }

    Dialog {
        id: groupEditor
        property string groupId: ""
        property bool creating: false
        title: creating ? root.tr("Create group") : root.tr("Rename group")
        anchors.centerIn: parent
        modal: true
        width: Math.min(420, root.width - 40)
        standardButtons: Dialog.Save | Dialog.Cancel
        onOpened: {
            root.translateButtons(groupEditor);
            standardButton(Dialog.Save).enabled = Qt.binding(() => groupNameInput.text.trim() !== "");
            groupNameInput.forceActiveFocus();
            groupNameInput.selectAll();
        }
        onAccepted: {
            const name = groupNameInput.text.trim();
            if (name === "") return;
            if (creating) {
                if (bookmarkGroups.count >= 100) return;
                let id = "group-" + Date.now();
                while (root.groupIndex(id) >= 0) id += "x";
                bookmarkGroups.append({id:id, name:name, parentId:groupId, expanded:true});
            } else {
                const index = root.groupIndex(groupId);
                if (index < 0) return;
                bookmarkGroups.setProperty(index, "name", name);
            }
            root.rebuildBookmarkTree();
            root.saveWorkspace();
        }
        TextField { id: groupNameInput; width: parent.width; maximumLength: 256; placeholderText: root.tr("Group name (required)") }
    }
    Dialog {
        id: bookmarkGroupConfirm
        property string groupId: ""
        title: root.tr("Run bookmarked commands?")
        anchors.centerIn: parent
        modal: true
        width: Math.min(480, root.width - 40)
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: root.translateButtons(bookmarkGroupConfirm)
        onAccepted: root.openBookmarkGroup(groupId, true)
        Label { width: parent.width; wrapMode: Text.WordWrap; text: root.tr("This group contains custom commands. Opening all bookmarks runs these commands again.") }
    }

    Dialog {
        id: bookmarksDialog
        title: root.tr("Bookmarks")
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        height: Math.min(480, root.height - 40)
        standardButtons: Dialog.Close
        onOpened: {
            root.translateButtons(bookmarksDialog);
            root.rebuildBookmarkTree();
            if (bookmarkList.currentIndex < 0 && bookmarkTree.count > 0) bookmarkList.currentIndex = 0;
        }
        ColumnLayout {
            anchors.fill: parent
            Label {
                text: root.tr("Double-click a group to open all its logs. Use the arrow to expand or collapse.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            ListView {
                id: bookmarkList
                model: bookmarkTree
                clip: true
                Layout.fillWidth: true
                Layout.fillHeight: true
                ScrollBar.vertical: ScrollBar {}
                delegate: ItemDelegate {
                    id: bookmarkItem
                    required property int index
                    required property string bookmarkTitle
                    required property string logPath
                    required property string source
                    required property bool remote
                    required property string connectionId
                    required property bool isGroup
                    required property string nodeId
                    required property int sourceIndex
                    required property int depth
                    required property bool expanded
                    property alias contextMenu: bookmarkMenu
                    readonly property string connectionName: remote ? (root.profileById(connectionId).name || "") : ""
                    readonly property string sourcePath: isGroup ? "" : (source || "file") + ":" + logPath
                    function selectBookmark(button) {
                        bookmarkList.currentIndex = index;
                        if (button === Qt.RightButton) bookmarkMenu.popup();
                    }
                    function activateBookmark(button) {
                        if (button === Qt.LeftButton) {
                            if (isGroup) root.openBookmarkGroup(nodeId);
                            else root.openBookmark(sourceIndex);
                        }
                    }
                    width: bookmarkList.width
                    highlighted: bookmarkList.currentIndex === index
                    contentItem: RowLayout {
                        spacing: 12
                        Label {
                            objectName: "bookmarkTitleLabel"
                            text: (bookmarkItem.isGroup ? "▣ " : "") + bookmarkItem.bookmarkTitle
                            textFormat: Text.PlainText
                            font.bold: true
                            elide: Text.ElideRight
                            wrapMode: Text.NoWrap
                            maximumLineCount: 1
                            Layout.minimumWidth: 0
                            Layout.preferredWidth: Math.max(0, bookmarkList.width * 0.34 - bookmarkItem.leftPadding)
                            Layout.maximumWidth: bookmarkItem.isGroup ? Infinity : Layout.preferredWidth
                            Layout.fillWidth: bookmarkItem.isGroup
                        }
                        Label {
                            objectName: "bookmarkConnectionLabel"
                            visible: !bookmarkItem.isGroup
                            text: bookmarkItem.connectionName
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            wrapMode: Text.NoWrap
                            maximumLineCount: 1
                            Layout.minimumWidth: 0
                            Layout.preferredWidth: bookmarkList.width * 0.22
                            Layout.maximumWidth: Layout.preferredWidth
                        }
                        Label {
                            objectName: "bookmarkPathLabel"
                            visible: !bookmarkItem.isGroup
                            text: bookmarkItem.sourcePath
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            wrapMode: Text.NoWrap
                            maximumLineCount: 1
                            Layout.minimumWidth: 0
                            Layout.fillWidth: true
                        }
                    }
                    HoverHandler { id: bookmarkHover }
                    ToolTip {
                        id: bookmarkTooltip
                        objectName: "bookmarkRowTooltip"
                        visible: bookmarkHover.hovered
                        delay: 500
                        text: bookmarkItem.bookmarkTitle + (bookmarkItem.isGroup ? "" : "\n"
                            + (bookmarkItem.connectionName ? bookmarkItem.connectionName + "\n" : "") + bookmarkItem.sourcePath)
                        width: Math.min(600, implicitWidth)
                        contentItem: Label { text: bookmarkTooltip.text; textFormat: Text.PlainText; wrapMode: Text.WrapAnywhere }
                    }
                    MouseArea {
                        objectName: "bookmarkRowMouse"
                        anchors.fill: parent
                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                        onClicked: mouse => {
                            if (mouse.button === Qt.LeftButton && bookmarkItem.isGroup && mouse.x < 24 + bookmarkItem.depth * 20)
                                root.toggleBookmarkGroup(bookmarkItem.nodeId);
                            else bookmarkItem.selectBookmark(mouse.button);
                        }
                        onDoubleClicked: mouse => bookmarkItem.activateBookmark(mouse.button)
                    }
                    leftPadding: 30 + depth * 20
                    Label {
                        visible: bookmarkItem.isGroup
                        x: 6 + bookmarkItem.depth * 20
                        anchors.verticalCenter: parent.verticalCenter
                        text: bookmarkItem.expanded ? "▼" : "▶"
                    }
                    Menu {
                        id: bookmarkMenu
                        MenuItem { text: root.tr("Open all logs"); visible: bookmarkItem.isGroup; height: visible ? implicitHeight : 0; onTriggered: root.openBookmarkGroup(bookmarkItem.nodeId) }
                        MenuItem { text: root.tr("Create subgroup"); visible: bookmarkItem.isGroup; height: visible ? implicitHeight : 0; enabled: bookmarkGroups.count < 100; onTriggered: root.editBookmarkGroup(bookmarkItem.nodeId, true) }
                        MenuItem { text: root.tr("Edit…"); enabled: !bookmarkItem.isGroup || bookmarkItem.nodeId !== ""; onTriggered: bookmarkItem.isGroup ? root.editBookmarkGroup(bookmarkItem.nodeId) : root.editBookmark(bookmarkItem.sourceIndex) }
                        MenuItem { text: root.tr("Delete"); enabled: !bookmarkItem.isGroup || bookmarkItem.nodeId !== ""; onTriggered: bookmarkItem.isGroup ? root.deleteBookmarkGroup(bookmarkItem.nodeId) : root.deleteBookmark(bookmarkItem.sourceIndex) }
                    }
                }
                Keys.onReturnPressed: if (currentItem) currentItem.activateBookmark(Qt.LeftButton)
                Keys.onEnterPressed: if (currentItem) currentItem.activateBookmark(Qt.LeftButton)
            }
            Label {
                visible: bookmarks.count === 0
                text: root.tr("Right-click a log tab to register a bookmark.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label {
                text: root.tr("Opening a bookmark starts new log collection. Commands run again. Existing archives are kept.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label {
                visible: bookmarks.count >= 100
                text: root.tr("Up to 100 bookmarks can be saved.")
                Layout.fillWidth: true
            }
            RowLayout {
                Button { text: root.tr("Create group"); enabled: bookmarkGroups.count < 100; onClicked: root.editBookmarkGroup("", true) }
                Button {
                    text: root.tr("Open bookmark")
                    enabled: !!bookmarkList.currentItem
                    onClicked: bookmarkList.currentItem.activateBookmark(Qt.LeftButton)
                }
                Button {
                    text: root.tr("Edit…")
                    enabled: !!bookmarkList.currentItem && (!bookmarkList.currentItem.isGroup || bookmarkList.currentItem.nodeId !== "")
                    onClicked: bookmarkList.currentItem.isGroup ? root.editBookmarkGroup(bookmarkList.currentItem.nodeId) : root.editBookmark(bookmarkList.currentItem.sourceIndex)
                }
                Button {
                    text: root.tr("Delete")
                    enabled: !!bookmarkList.currentItem && (!bookmarkList.currentItem.isGroup || bookmarkList.currentItem.nodeId !== "")
                    onClicked: bookmarkList.currentItem.isGroup ? root.deleteBookmarkGroup(bookmarkList.currentItem.nodeId) : root.deleteBookmark(bookmarkList.currentItem.sourceIndex)
                }
            }
        }
    }

    Dialog {
        id: closeTabDialog
        property int tabIndex: -1
        property string closeMode: "single"
        function requestClose(index, mode = "single") {
            if (index < 0 || index >= logs.count) return;
            tabIndex = index;
            closeMode = mode;
            open();
        }
        title: closeMode === "all" ? root.tr("Close all tabs?") : closeMode === "others" ? root.tr("Close other tabs?") : root.tr("Close log tab?")
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.CloseOnEscape
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: root.translateButtons(closeTabDialog)
        onAccepted: { if (tabIndex >= 0 && tabIndex < logs.count) root.closeTabs(closeMode, tabIndex); }
        Label { text: root.tr("Closing stops log collection. Saved archives are kept."); wrapMode: Text.WordWrap }
    }

    Dialog {
        id: editTabDialog
        property int tabIndex: -1
        property bool remote: false
        title: root.tr("Edit log and reconnect")
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        standardButtons: Dialog.Ok | Dialog.Cancel
        function edit(index) {
            const row = logs.get(index);
            tabIndex = index;
            remote = row.remote;
            editLogTitle.text = row.logTitle;
            editSource.currentIndex = ["file", "docker", "custom", "syslog-udp"].indexOf(row.source);
            editLogPath.text = row.logPath;
            editEncoding.currentIndex = root.encodingOptions.indexOf(row.encoding);
            editProfile.currentIndex = -1;
            for (let i = 0; i < connectionProfiles.count; ++i) {
                if (connectionProfiles.get(i).id === row.connectionId) editProfile.currentIndex = i;
            }
            editElevation.currentIndex = row.elevation;
            editRunUser.text = row.runUser;
            editInitial.value = row.initial;
            editCapacity.value = row.capacity;
            open();
        }
        onOpened: {
            root.translateButtons(editTabDialog);
            standardButton(Dialog.Ok).enabled = Qt.binding(() => editLogPath.text.trim() !== ""
                && (editSource.currentIndex !== 2 || Array.from(editLogPath.text).length <= 8192)
                && (!remote || editProfile.currentIndex >= 0));
        }
        onAccepted: {
            const index = tabIndex;
            if (index < 0 || index >= logs.count) return;
            const previous = logs.get(index);
            const encodingOnly = editSource.currentIndex !== 3 && previous.source === ["file", "docker", "custom", "syslog-udp"][editSource.currentIndex]
                && previous.logPath === editLogPath.text
                && (!remote || previous.connectionId === connectionProfiles.get(editProfile.currentIndex).id)
                && previous.elevation === editElevation.currentIndex && previous.runUser === editRunUser.text
                && previous.initial === editInitial.value && previous.capacity === editCapacity.value
                && previous.encoding !== editEncoding.currentText;
            logs.setProperty(index, "logTitle", editLogTitle.text.trim() || logs.get(index).logTitle);
            logs.setProperty(index, "source", ["file", "docker", "custom", "syslog-udp"][editSource.currentIndex]);
            logs.setProperty(index, "logPath", editLogPath.text);
            logs.setProperty(index, "encoding", editSource.currentIndex === 3 ? "UTF-8" : editEncoding.currentText);
            if (remote) logs.setProperty(index, "connectionId", connectionProfiles.get(editProfile.currentIndex).id);
            logs.setProperty(index, "elevation", editElevation.currentIndex);
            logs.setProperty(index, "runUser", editRunUser.text);
            logs.setProperty(index, "initial", editInitial.value);
            logs.setProperty(index, "capacity", editCapacity.value);
            if (encodingOnly) {
                // qmllint disable missing-property
                pageRepeater.itemAt(index).backend.set_encoding(editEncoding.currentText);
                // qmllint enable missing-property
                root.scheduleSave();
            } else root.restartTab(index);
        }
        ColumnLayout {
            anchors.fill: parent
            TextField { id: editLogTitle; placeholderText: root.tr("Tab name (optional)"); Layout.fillWidth: true }
            ComboBox {
                id: editSource
                model: editTabDialog.remote
                    ? [root.tr("File (tail)"), root.tr("Docker container"), root.tr("Custom command")]
                    : [root.tr("File (tail)"), root.tr("Docker container"), root.tr("Custom command"), root.tr("Syslog (UDP)")]
                Layout.fillWidth: true
                onActivated: { if (!editTabDialog.remote && currentIndex === 1) currentIndex = 0; }
            }
            Label { visible: editTabDialog.remote; text: root.tr("Saved connection") }
            ComboBox { id: editProfile; visible: editTabDialog.remote; model: connectionProfiles; textRole: "name"; Layout.fillWidth: true }
            Label { text: editSource.currentIndex === 3 ? root.tr("Listen endpoint (IP:port)") : root.tr("Log path, container or command") }
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: editSource.currentIndex === 2 ? 120 : 60
                TextArea { id: editLogPath; wrapMode: TextEdit.Wrap; selectByMouse: true }
            }
            RowLayout {
                Label { text: root.tr("Input encoding") }
                ComboBox { id: editEncoding; model: root.encodingOptions; Layout.fillWidth: true; enabled: editSource.currentIndex !== 3 }
            }
            RowLayout {
                visible: editTabDialog.remote
                Label { text: root.tr("Privilege escalation") }
                ComboBox { id: editElevation; model: [root.tr("None"), "sudo", root.tr("su (login)")] }
                TextField { id: editRunUser; enabled: editElevation.currentIndex !== 0; placeholderText: root.tr("Run as user"); Layout.fillWidth: true }
            }
            RowLayout {
                visible: editSource.currentIndex < 2
                Label { text: root.tr("Initial tail lines") }
                SpinBox { id: editInitial; from: 0; to: 1000000; editable: true }
            }
            Label { text: root.tr("Ring buffer: last 200,000 lines / 25MiB. Display: last 2,000 lines / 256KiB. Collection continues when full."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
            SpinBox { id: editCapacity; visible: false; from: 1; to: 10240 }
            Label { text: root.tr("Reconnection starts a new archive. Existing archives are kept. Commands will run again."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
    }

    Dialog {
        id: localCommandDialog
        title: root.tr("Local command")
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: {
            root.translateButtons(localCommandDialog);
            standardButton(Dialog.Ok).enabled = Qt.binding(() => localCommandInput.text.trim() !== "" && Array.from(localCommandInput.text).length <= 8192);
        }
        onAccepted: {
            root.addLog(localCommandInput.text, "custom", localCommandTitle.text.trim() || root.tr("Local command"), localCommandEncoding.currentText);
        }
        ColumnLayout {
            anchors.fill: parent
            TextField { id: localCommandTitle; placeholderText: root.tr("Tab name (optional)"); Layout.fillWidth: true }
            RowLayout {
                Label { text: root.tr("Input encoding") }
                ComboBox { id: localCommandEncoding; model: root.encodingOptions; Layout.fillWidth: true }
            }
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: 140
                TextArea { id: localCommandInput; placeholderText: "docker logs -f --tail 50 web"; wrapMode: TextEdit.Wrap; selectByMouse: true }
            }
            Label { text: root.tr("Runs locally with /bin/sh. Saves stdout and stderr. Saved commands run again on startup."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
    }

    ListModel {
        id: logs
    }

    Dialog {
        id: syslogDialog
        title: root.tr("Syslog (UDP)")
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: {
            root.translateButtons(syslogDialog);
            standardButton(Dialog.Ok).enabled = Qt.binding(() => syslogEndpoint.text.trim() !== "");
        }
        onAccepted: root.addLog(syslogEndpoint.text.trim(), "syslog-udp",
            syslogTitle.text.trim() || "Syslog UDP " + syslogEndpoint.text.trim(), "UTF-8")
        ColumnLayout {
            anchors.fill: parent
            TextField { id: syslogTitle; placeholderText: root.tr("Tab name (optional)"); Layout.fillWidth: true }
            Label { text: root.tr("Listen endpoint (IP:port)") }
            TextField { id: syslogEndpoint; text: "127.0.0.1:1514"; placeholderText: "0.0.0.0:1514 / [::]:1514"; Layout.fillWidth: true; selectByMouse: true }
            Label {
                text: root.tr("Receives raw UTF-8 syslog over UDP. Use 0.0.0.0:1514 for LAN devices. No authentication; restrict access with your firewall. Saved listeners restart on startup.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    Dialog {
        id: profilesDialog
        title: root.tr("Connections")
        onOpened: root.translateButtons(profilesDialog)
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            ComboBox {
                id: profileSelection
                model: connectionProfiles
                textRole: "name"
                Layout.fillWidth: true
            }
            Label {
                text: root.tr("Changes apply on the next connection. Connections used by tabs or bookmarks cannot be deleted.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            RowLayout {
                Button { text: root.tr("Add…"); onClicked: profileEditor.edit(-1) }
                Button {
                    text: root.tr("Edit…")
                    enabled: profileSelection.currentIndex >= 0
                    onClicked: profileEditor.edit(profileSelection.currentIndex)
                }
                Button {
                    text: root.tr("Delete")
                    enabled: profileSelection.currentIndex >= 0 && !root.profileUsed(connectionProfiles.get(profileSelection.currentIndex).id)
                    onClicked: {
                        connectionProfiles.remove(profileSelection.currentIndex);
                        root.scheduleSave();
                    }
                }
            }
        }
    }

    Dialog {
        id: profileEditor
        property int editIndex: -1
        function edit(index) {
            editIndex = index;
            const p = index >= 0 ? connectionProfiles.get(index) : {name: "", host: "", user: "", port: 0, key: ""};
            profileName.text = p.name;
            profileHost.text = p.host;
            profileUser.text = p.user;
            profilePort.value = p.port;
            profileKey.text = p.key;
            open();
        }
        title: editIndex < 0 ? root.tr("Add connection") : root.tr("Edit connection")
        anchors.centerIn: parent
        modal: true
        width: Math.min(600, root.width - 40)
        standardButtons: Dialog.Save | Dialog.Cancel
        onOpened: {
            root.translateButtons(profileEditor);
            standardButton(Dialog.Save).enabled = Qt.binding(() => profileName.text.trim() !== "" && profileHost.text.trim() !== "" && !profileHost.text.trim().startsWith("-") && !/\s/.test(profileHost.text.trim()));
        }
        onAccepted: {
            const p = {id: editIndex >= 0 ? connectionProfiles.get(editIndex).id : "connection-" + Date.now() + "-" + Math.random().toString(36).slice(2), name: profileName.text.trim(), host: profileHost.text.trim(), user: profileUser.text.trim(), port: profilePort.value, key: profileKey.text.trim()};
            if (editIndex >= 0)
                connectionProfiles.set(editIndex, p);
            else
                connectionProfiles.append(p);
            profileSelection.currentIndex = editIndex >= 0 ? editIndex : connectionProfiles.count - 1;
            if (sshProfile.currentIndex < 0)
                sshProfile.currentIndex = 0;
            root.scheduleSave();
        }
        ColumnLayout {
            anchors.fill: parent
            TextField { id: profileName; placeholderText: root.tr("Connection name (required)"); Layout.fillWidth: true }
            TextField { id: profileHost; placeholderText: root.tr("Host name / IP address (required)"); Layout.fillWidth: true }
            TextField { id: profileUser; placeholderText: root.tr("User (blank = OS user)"); Layout.fillWidth: true }
            Label { text: root.tr("Port (0 = 22)") }
            SpinBox { id: profilePort; from: 0; to: 65535; editable: true }
            TextField { id: profileKey; placeholderText: root.tr("Private key path (blank = standard ~/.ssh keys)"); Layout.fillWidth: true }
            Label { text: root.tr("Registration does not connect. Passwords are not stored in configuration."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
    }

    Dialog {
        id: connectionAuth
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(600, root.width - 40)
        modal: true
        title: connectionManager.authConfirmation ? root.tr("Verify SSH host key") : root.tr("Shared SSH authentication")
        standardButtons: Dialog.Ok | Dialog.Cancel
        closePolicy: Popup.NoAutoClose
        onOpened: { root.translateButtons(connectionAuth); connectionSecret.clear(); connectionRemember.checked = false; connectionSecret.forceActiveFocus(); }
        onClosed: Qt.callLater(() => { if (connectionManager.authPrompt !== "" && !connectionAuth.visible) connectionAuth.open(); })
        onAccepted: {
            connectionManager.answer_auth(connectionManager.authConfirmation ? "yes" : connectionSecret.text, true, connectionRemember.checked);
            connectionSecret.clear();
        }
        onRejected: { connectionManager.answer_auth("", false, false); connectionSecret.clear(); }
        ColumnLayout {
            anchors.fill: parent
            Label { text: connectionManager.authPrompt; wrapMode: Text.WrapAnywhere; Layout.fillWidth: true }
            Label { visible: connectionManager.authConfirmation; text: root.tr("Verify the fingerprint through a trusted channel before accepting."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
            TextField { id: connectionSecret; visible: !connectionManager.authConfirmation; echoMode: TextInput.Password; Layout.fillWidth: true }
            CheckBox { id: connectionRemember; visible: connectionManager.authStorable; text: root.tr("Save in OS credential store") }
            Button { visible: connectionManager.authStorable; text: root.tr("Delete saved credentials"); onClicked: connectionManager.forget_credential() }
            Label { text: connectionManager.credentialStatus; wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
    }

    Dialog {
        id: settingsDialog
        title: root.tr("Settings")
        anchors.centerIn: parent
        modal: true
        standardButtons: Dialog.Save | Dialog.Cancel
        onOpened: {
            root.translateButtons(settingsDialog);
            defaultLines.value = settings.initialLines;
            defaultCapacity.value = settings.capacityMiB;
            appearanceSelection.currentIndex = appearanceSelection.codes.indexOf(settings.appearance);
        }
        onAccepted: {
            settings.initialLines = defaultLines.value;
            settings.capacityMiB = defaultCapacity.value;
            root.scheduleSave();
        }
        ColumnLayout {
            RowLayout {
                Label { text: root.tr("Appearance") }
                ComboBox {
                    id: appearanceSelection
                    model: [root.tr("Auto (system)"), root.tr("Light"), root.tr("Dark")]
                    property var codes: ["auto", "light", "dark"]
                    currentIndex: Math.max(0, codes.indexOf(settings.appearance))
                    onActivated: settings.appearance = codes[currentIndex]
                }
            }
            RowLayout {
                Label { text: root.tr("Display language") }
                ComboBox {
                    id: languageSelection
                    model: ["English", "日本語", "简体中文", "繁體中文", "한국어"]
                    property var codes: ["en", "ja", "zh-CN", "zh-TW", "ko"]
                    currentIndex: Math.max(0, codes.indexOf(translations.language))
                    onActivated: translations.select(codes[currentIndex])
                }
            }
            Label {
                text: root.tr("Applies to new tabs only")
            }
            Label {
                text: root.tr("Initial tail lines (0 = new output only)")
            }
            SpinBox {
                id: defaultLines
                from: 0
                to: 1000000
                editable: true
            }
            Label {
                text: root.tr("Ring buffer: last 200,000 lines / 25MiB. Display: last 2,000 lines / 256KiB. Collection continues when full.")
                wrapMode: Text.WordWrap
                Layout.maximumWidth: 480
            }
            SpinBox {
                id: defaultCapacity
                visible: false
                from: 1
                to: 10240
                editable: true
            }
            Label {
                text: root.tr("Logs are stored locally and may contain sensitive information.")
            }
        }
    }

    Dialog {
        id: sshDialog
        title: root.tr("New SSH log")
        anchors.centerIn: parent
        modal: true
        width: Math.min(620, root.width - 40)
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: {
            root.translateButtons(sshDialog);
            sshLines.value = settings.initialLines;
            if (sshProfile.currentIndex < 0 && connectionProfiles.count > 0)
                sshProfile.currentIndex = 0;
            standardButton(Dialog.Ok).enabled = Qt.binding(() => connectionProfiles.count > 0 && sshProfile.currentIndex >= 0 && (sshSource.currentValue !== "custom" || (sshCommand.text.trim() !== "" && Array.from(sshCommand.text).length <= 8192)));
        }
        onAccepted: {
            if (sshProfile.currentIndex < 0)
                return;
            const profile = connectionProfiles.get(sshProfile.currentIndex);
            logs.append({
                logPath: sshSource.currentValue === "custom" ? sshCommand.text : (sshSource.currentValue === "docker" ? sshContainer.text.trim() : sshPath.text),
                logTitle: profile.name + ":" + (sshSource.currentValue === "custom" ? (sshCommandTitle.text.trim() || root.tr("Custom")) : (sshSource.currentValue === "docker" ? "docker/" + sshContainer.text.trim() : sshPath.text.split("/").pop())),
                remote: true,
                unread: false,
                alert: false,
                trapText: "",
                trapCase: false,
                trapRulesJson: "[]",
                source: sshSource.currentValue,
                encoding: sshEncoding.currentText,
                connectionId: profile.id,
                elevation: sshElevation.currentIndex,
                runUser: sshRunUser.text,
                initial: sshLines.value,
                capacity: settings.capacityMiB,
                filterText: "",
                filterEnabled: true,
                filterRegex: false,
                filterCase: false,
                filterInvert: false,
                filterNumericJson: "null",
                searchText: "",
                searchRegex: false,
                searchCase: false,
                follow: true,
                analysisPresetId: "builtin-apache",
                analysisMergeFirstColumn: false,
                syslogBadges: false
            });
            tabs.currentIndex = logs.count - 1;
            root.scheduleSave();
        }
        ColumnLayout {
            anchors.fill: parent
            RowLayout {
                Label {
                    text: root.tr("Log source")
                }
                ComboBox {
                    id: sshSource
                    textRole: "label"
                    valueRole: "kind"
                    model: [
                        {
                            label: root.tr("File (tail)"),
                            kind: "file"
                        },
                        {
                            label: root.tr("Docker container"),
                            kind: "docker"
                        },
                        {
                            label: root.tr("Custom command"),
                            kind: "custom"
                        }
                    ]
                    Layout.fillWidth: true
                }
            }
            Label { text: root.tr("Saved connection") }
            RowLayout {
                Label { text: root.tr("Input encoding") }
                ComboBox { id: sshEncoding; model: root.encodingOptions; Layout.fillWidth: true }
            }
            Label {
                text: root.tr("Input is converted to UTF-8 for storage. CP932 is Windows Shift_JIS. LANG is unchanged.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            ComboBox {
                id: sshProfile
                model: connectionProfiles
                textRole: "name"
                Layout.fillWidth: true
            }
            Button {
                text: root.tr("Connections…")
                onClicked: profilesDialog.open()
            }
            TextField {
                id: sshPath
                visible: sshSource.currentValue === "file"
                placeholderText: root.tr("Absolute remote log path (required)")
                Layout.fillWidth: true
            }
            TextField {
                id: sshContainer
                visible: sshSource.currentValue === "docker"
                placeholderText: root.tr("Container name / ID (required), e.g. web-1")
                Layout.fillWidth: true
            }
            TextField {
                id: sshCommandTitle
                visible: sshSource.currentValue === "custom"
                placeholderText: root.tr("Tab name (optional)")
                Layout.fillWidth: true
            }
            ScrollView {
                visible: sshSource.currentValue === "custom"
                Layout.fillWidth: true
                Layout.preferredHeight: 120
                TextArea {
                    id: sshCommand
                    placeholderText: root.tr("Remote command (pipelines and multiple lines supported)")
                    wrapMode: TextEdit.Wrap
                    selectByMouse: true
                }
            }
            Label {
                visible: sshSource.currentValue === "docker"
                text: root.tr("Runs docker logs remotely. No Docker API port is needed. Saves stdout, stderr and Docker diagnostics.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label {
                visible: sshSource.currentValue === "custom"
                text: root.tr("Runs remotely with /bin/sh. Saves stdout and stderr (up to 8,192 command characters). Saved commands run again on startup.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            RowLayout {
                Label {
                    text: root.tr("Privilege escalation")
                }
                ComboBox {
                    id: sshElevation
                    model: [root.tr("None"), "sudo", root.tr("su (login)")]
                }
                TextField {
                    id: sshRunUser
                    text: "root"
                    placeholderText: root.tr("Run as user")
                    enabled: sshElevation.currentIndex !== 0
                    Layout.fillWidth: true
                }
            }
            Label {
                visible: sshElevation.currentIndex !== 0
                text: root.tr("Escalation passwords are entered separately. Requires Linux / macOS / BSD and a POSIX-compatible user shell. With a TTY, stderr is included in logs.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Label {
                text: root.tr("Initial tail lines")
                visible: sshSource.currentValue !== "custom"
            }
            SpinBox {
                id: sshLines
                visible: sshSource.currentValue !== "custom"
                from: 0
                to: 1000000
                editable: true
            }
            Label {
                text: root.tr("Enter passwords / key passphrases when requested. Saving to the OS credential store is optional.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    FileDialog {
        id: fileDialog
        title: root.tr("Select logs to follow")
        fileMode: FileDialog.OpenFiles
        onAccepted: {
            for (const url of selectedFiles) {
                const path = root.localFilePath(url);
                root.addLog(path);
            }
        }
    }

    function hasLocalFileUrls(urls) {
        for (const url of urls)
            if (/^file:\/\//i.test(url.toString())) return true;
        return false;
    }
    function dropLocalFiles(drop) {
        if (!drop.hasUrls || !hasLocalFileUrls(drop.urls)) {
            drop.accepted = false;
            return;
        }
        for (const url of drop.urls) {
            if (/^file:\/\//i.test(url.toString()))
                root.addLog(root.localFilePath(url));
        }
        // Opening a log must never request moving the original file.
        drop.accept(Qt.CopyAction);
    }
    DropArea {
        id: fileDropArea
        parent: root.contentItem.parent
        anchors.fill: parent
        z: 100
        onEntered: (drag) => {
            drag.accepted = drag.hasUrls && root.hasLocalFileUrls(drag.urls);
        }
        onDropped: (drop) => root.dropLocalFiles(drop)
    }

    Shortcut {
        sequences: [StandardKey.Open]
        onActivated: fileDialog.open()
    }
    Shortcut {
        sequences: [StandardKey.Close]
        onActivated: {
            if (logs.count > 0)
                closeTabDialog.requestClose(tabs.currentIndex);
        }
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            ToolButton {
                text: root.tr("Open logs…")
                onClicked: fileDialog.open()
            }
            ComboBox {
                id: localEncoding
                model: root.encodingOptions
                ToolTip.text: root.tr("Encoding for new local logs")
                ToolTip.visible: hovered
            }
            ToolButton {
                text: root.tr("SSH logs…")
                onClicked: sshDialog.open()
            }
            ToolButton {
                text: root.tr("Local command…")
                onClicked: localCommandDialog.open()
            }
            ToolButton {
                text: root.tr("Syslog…")
                onClicked: syslogDialog.open()
            }
            ToolButton {
                text: root.tr("Connections…")
                onClicked: profilesDialog.open()
            }
            ToolButton {
                text: root.tr("Bookmarks…")
                onClicked: bookmarksDialog.open()
            }
            ToolButton {
                text: root.tr("Settings…")
                onClicked: settingsDialog.open()
            }
            Label {
                Layout.fillWidth: true
            }
        }
    }
    footer: Label {
        id: workspaceFooter
        function statusText(row, statuses) {
            const connection = row && row.remote ? statuses[row.connectionId] : "";
            return connection ? root.tr("SSH transport") + ": " + connection : "";
        }
        text: workspace.error || statusText(tabs.currentIndex >= 0 && tabs.currentIndex < logs.count ? logs.get(tabs.currentIndex) : null, connectionManager.statuses || {})
        visible: text !== ""
        color: workspace.error ? "red" : palette.text
        wrapMode: Text.WrapAnywhere
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        TabBar {
            id: tabs
            // The layout supplies the bar width; do not derive it from tab widths.
            implicitWidth: 0
            onCurrentIndexChanged: {
                if (root.movingTab) return;
                if (currentIndex >= 0 && currentIndex < logs.count) {
                    logs.setProperty(currentIndex, "unread", false);
                    logs.setProperty(currentIndex, "alert", false);
                }
                root.scheduleSave();
            }
            Layout.fillWidth: true
            visible: logs.count > 0
            Repeater {
                id: tabRepeater
                model: logs
                TabButton {
                    id: tab
                    required property string logTitle
                    required property int index
                    required property bool unread
                    required property bool alert
                    property bool connected: false
                    property alias statusDot: notificationDot
                    property alias contextMenu: tabMenu
                    opacity: tabDrag.active ? 0.55 : 1
                    text: logTitle
                    width: Math.min(Math.max(100, implicitWidth), 260, tabs.availableWidth)
                    rightPadding: 36
                    implicitHeight: Math.max(implicitBackgroundHeight + topInset + bottomInset,
                                             implicitContentHeight + topPadding + bottomPadding) + 6
                    background: Rectangle {
                        implicitHeight: 21
                        color: tab.checked ? Qt.lighter(tab.palette.button, 1.50) : (tab.hovered ? Qt.lighter(tab.palette.button, 1.08) : tab.palette.button)
                        border.color: tab.palette.mid
                        topLeftRadius: 8
                        topRightRadius: 8
                        bottomLeftRadius: 0
                        bottomRightRadius: 0
                        antialiasing: true
                    }
                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        onTapped: tabMenu.popup()
                    }
                    DragHandler {
                        id: tabDrag
                        target: null
                        acceptedButtons: Qt.LeftButton
                        yAxis.enabled: false
                        property int sourceIndex: -1
                        onActiveChanged: {
                            if (active) sourceIndex = tab.index;
                            else if (sourceIndex >= 0) {
                                root.dropTab(sourceIndex, centroid.scenePosition.x, centroid.scenePosition.y);
                                sourceIndex = -1;
                            }
                        }
                    }
                    Menu {
                        id: tabMenu
                        MenuItem { text: root.tr("Edit and reconnect…"); onTriggered: editTabDialog.edit(tab.index) }
                        MenuItem { text: root.tr("Reconnect / run again"); onTriggered: root.restartTab(tab.index) }
                        MenuSeparator {}
                        MenuItem {
                            text: root.tr("Register bookmark")
                            enabled: bookmarks.count < 100
                            onTriggered: root.registerBookmark(tab.index)
                        }
                        MenuSeparator {}
                        MenuItem { text: root.tr("Close"); onTriggered: closeTabDialog.requestClose(tab.index) }
                        MenuItem { text: root.tr("Close other tabs"); enabled: logs.count > 1; onTriggered: closeTabDialog.requestClose(tab.index, "others") }
                        MenuItem { text: root.tr("Close all tabs"); onTriggered: closeTabDialog.requestClose(tab.index, "all") }
                    }
                    contentItem: RowLayout {
                        spacing: 6
                        Label {
                            text: tab.logTitle
                            color: tab.palette.buttonText
                            elide: Text.ElideRight
                            horizontalAlignment: Text.AlignHCenter
                            Layout.fillWidth: true
                        }
                        Rectangle {
                            id: notificationDot
                            visible: !tab.connected || tab.unread
                            implicitWidth: 8
                            implicitHeight: 8
                            radius: 4
                            color: tab.connected ? "#35c66b" : "#e05252"
                            Accessible.name: !tab.connected ? root.tr("Log source disconnected") : (tab.alert ? root.tr("New logs with trapped text") : root.tr("Unread log updates"))
                            SequentialAnimation on opacity {
                                running: tab.connected && tab.alert
                                loops: Animation.Infinite
                                NumberAnimation {
                                    to: 0.2
                                    duration: 450
                                }
                                NumberAnimation {
                                    to: 1
                                    duration: 450
                                }
                                onRunningChanged: {
                                    if (!running)
                                        notificationDot.opacity = 1;
                                }
                            }
                        }
                    }
                    ToolButton {
                        objectName: "closeTabButton"
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: 30
                        text: "×"
                        onClicked: closeTabDialog.requestClose(tab.index)
                    }
                }
            }
        }

        Label {
            visible: logs.count === 0
            text: root.tr("Select files using “Open logs…”")
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            Layout.fillWidth: true
            Layout.fillHeight: true
        }

        StackLayout {
            id: pages
            currentIndex: tabs.currentIndex
            visible: logs.count > 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            Repeater {
                id: pageRepeater
                model: logs
                Item {
                    id: page
                    readonly property var logText: logDocument.item
                    required property string logPath
                    required property bool remote
                    required property string source
                    required property string encoding
                    required property string connectionId
                    required property int elevation
                    required property string runUser
                    required property int initial
                    required property int capacity
                    required property int index
                    readonly property bool viewActive: index === tabs.currentIndex
                    onViewActiveChanged: {
                        if (viewReady) backend.set_view_active(viewActive);
                    }
                    required property string filterText
                    required property bool filterEnabled
                    required property bool filterRegex
                    required property bool filterCase
                    required property bool filterInvert
                    required property string filterNumericJson
                    property var filterNumeric: JSON.parse(filterNumericJson)
                    function numericFilterCondition() {
                        return filterNumericEnabled.checked ? {capture:filterCapture.value, operator:filterOperator.currentText, value:filterValue.text} : null;
                    }
                    function applyFilter() {
                        backend.set_numeric_filter(filterEnable.checked ? filterInput.text : "",
                            filterRegex.checked, filterCase.checked,
                            filterEnable.checked && filterInvert.checked,
                            filterEnable.checked ? page.numericFilterCondition() : null);
                    }
                    required property string searchText
                    required property bool searchRegex
                    required property bool searchCase
                    required property bool follow
                    required property bool syslogBadges
                    required property string analysisPresetId
                    required property bool analysisMergeFirstColumn
                    required property string trapText
                    required property bool trapCase
                    required property string trapRulesJson
                    property var trapRules: JSON.parse(trapRulesJson)
                    property string trapError: ""
                    function commitTrapRules(rules) {
                        const error = backend.set_trap_rules(rules);
                        if (error !== "") { trapError = error; return false; }
                        logs.setProperty(index, "trapRulesJson", JSON.stringify(rules));
                        logs.setProperty(index, "alert", false);
                        trapError = "";
                        root.scheduleSave();
                        return true;
                    }
                    function addTrap() {
                        const rules = page.trapRules.slice();
                        rules.push({text: trapInput.text, regex: trapRegex.checked, ignoreCase: trapCase.checked, color: "#35c66b"});
                        if (commitTrapRules(rules)) trapInput.clear();
                    }
                    function removeTrap(tagIndex) {
                        const rules = page.trapRules.slice();
                        rules.splice(tagIndex, 1);
                        page.commitTrapRules(rules);
                    }
                    function setTrapEnabled(tagIndex, enabled) {
                        const rules = page.trapRules.slice();
                        rules[tagIndex] = Object.assign({}, rules[tagIndex], {enabled: enabled});
                        return page.commitTrapRules(rules);
                    }
                    function editTrap(tagIndex, selectedText = "") {
                        trapEditor.tagIndex = tagIndex;
                        const rule = tagIndex >= 0 ? page.trapRules[tagIndex]
                            : {text: selectedText, regex: false, ignoreCase: false, color: "#35c66b"};
                        editTrapText.text = rule.text;
                        editTrapEnabled.checked = rule.enabled !== false;
                        editTrapRegex.checked = rule.regex;
                        editTrapCase.checked = rule.ignoreCase;
                        editTrapColor.text = rule.color;
                        editTrapNumeric.checked = !!rule.numeric;
                        editTrapCapture.value = rule.numeric ? rule.numeric.capture : 1;
                        editTrapOperator.currentIndex = Math.max(0, editTrapOperator.model.indexOf(rule.numeric ? rule.numeric.operator : ">"));
                        editTrapValue.text = rule.numeric ? rule.numeric.value.toString() : "100";
                        editTrapBackground.checked = !!rule.background;
                        editTrapOpacity.value = rule.opacity === undefined ? 25 : rule.opacity;
                        editTrapScope.currentIndex = Math.max(0, editTrapScope.codes.indexOf(rule.scope || "match"));
                        trapEditor.error = "";
                        trapEditor.open();
                    }
                    property alias backend: backend
                    function testSyslogLabels(enabled) {
                        syslogBadgesToggle.checked = enabled;
                        syslogBadgesToggle.toggled();
                    }
                    function restartSource() {
                        backend.stop();
                        backend.set_syslog_badges(page.source === "syslog-udp" && page.syslogBadges);
                        backend.set_encoding(page.encoding);
                        if (page.remote)
                            backend.connect_profile(root.profileById(page.connectionId), page.logPath, page.initial, page.capacity, page.elevation, page.runUser, page.source);
                        else if (page.source === "custom")
                            backend.run_local(page.logPath, page.capacity);
                        else if (page.source === "syslog-udp")
                            backend.listen_syslog(page.logPath, page.capacity);
                        else
                            backend.open(page.logPath, page.initial, page.capacity);
                        page.applyFilter();
                        backend.search(searchInput.text, searchRegex.checked, searchCase.checked);
                        backend.set_trap_rules(page.trapRules);
                    }
                    function testOptionsPanel() {
                        const before = [filterInput.text, searchInput.text, trapInput.text];
                        if (optionsToggle.checked || filterRow.visible || searchRow.visible || querySummary.visible || trapRow.visible)
                            return false;
                        filterInput.text = "smoke-filter";
                        searchInput.text = "smoke-search";
                        trapInput.text = "smoke-trap";
                        optionsToggle.toggle();
                        if (!filterRow.visible || !searchRow.visible || !querySummary.visible || !trapRow.visible)
                            return false;
                        const numericEnabled = filterNumericEnabled.checked;
                        if (filterNumericEnabled.parent !== filterRow || !filterNumericEnabled.visible)
                            return false;
                        for (const checked of [false, true, false]) {
                            filterNumericEnabled.checked = checked;
                            if (filterNumericRow.visible !== checked
                                || filterCaptureLabel.visible !== checked || filterCapture.visible !== checked
                                || filterOperator.visible !== checked || filterValue.visible !== checked)
                                return false;
                        }
                        filterNumericEnabled.checked = numericEnabled;
                        optionsToggle.toggle();
                        const retained = !filterRow.visible && !filterNumericRow.visible && !searchRow.visible && !querySummary.visible && !trapRow.visible
                            && filterInput.text === "smoke-filter" && searchInput.text === "smoke-search" && trapInput.text === "smoke-trap"
                            && optionsToggle.text.includes(root.tr("Filter enabled"))
                            && (page.trapRules.length === 0 || optionsToggle.text.includes(root.tr("Text trap enabled")));
                        filterInput.text = before[0];
                        searchInput.text = before[1];
                        trapInput.text = before[2];
                        return retained;
                    }
                    function testManualScroll() {
                        scroll.pauseFollow();
                        return !follow.checked;
                    }
                    function testTrapTags() {
                        const before = page.trapRules.slice();
                        const inputBefore = trapInput.text;
                        trapInput.text = "ERROR";
                        trapRegex.checked = false;
                        trapCase.checked = false;
                        page.addTrap();
                        if (page.trapRules.length !== 1 || trapInput.text !== "") return false;
                        page.editTrap(0);
                        editTrapText.text = "[";
                        editTrapRegex.checked = true;
                        trapEditor.save();
                        if (trapEditor.error === "" || page.trapRules[0].text !== "ERROR") return false;
                        editTrapText.text = "ERROR.*?aaaa";
                        editTrapColor.text = "#ef5350";
                        trapEditor.save();
                        if (page.trapRules[0].text !== "ERROR.*?aaaa" || page.trapRules[0].color !== "#ef5350") return false;
                        if (!backend.highlight_result("ERROR xyz aaaa").includes("color:#ef5350")) return false;
                        if (!page.setTrapEnabled(0, false)) return false;
                        page.editTrap(0);
                        if (editTrapEnabled.checked || backend.highlight_result("ERROR xyz aaaa").includes("color:#ef5350")) return false;
                        editTrapEnabled.checked = true;
                        trapEditor.save();
                        if (!page.trapRules[0].enabled || !backend.highlight_result("ERROR xyz aaaa").includes("color:#ef5350")) return false;
                        selectionMenu.selectedText = "WARN[1]";
                        page.editTrap(-1, selectionMenu.selectedText);
                        if (editTrapText.text !== "WARN[1]" || editTrapRegex.checked) return false;
                        trapEditor.save();
                        if (page.trapRules.length !== 2 || page.trapRules[1].regex) return false;
                        for (let i = 2; i < 10; ++i) { trapInput.text = "tag" + i; page.addTrap(); }
                        if (page.trapRules.length !== 10) return false;
                        trapInput.text = "overflow";
                        page.addTrap();
                        if (page.trapRules.length !== 10 || page.trapError === "") return false;
                        page.removeTrap(0);
                        if (page.trapRules.length !== 9 || page.trapRules[0].text !== "WARN[1]") return false;
                        page.editTrap(0);
                        editTrapText.text = "cancelled";
                        trapEditor.close();
                        if (page.trapRules[0].text !== "WARN[1]") return false;
                        if (JSON.parse(logs.get(page.index).trapRulesJson).length !== 9) return false;
                        if (!page.commitTrapRules(before)) return false;
                        trapInput.text = inputBefore;
                        return true;
                    }
                    property bool viewReady: false
                    function saveView() {
                        if (root.restoring || !viewReady)
                            return;
                        logs.setProperty(index, "filterText", filterInput.text);
                        logs.setProperty(index, "filterEnabled", filterEnable.checked);
                        logs.setProperty(index, "filterRegex", filterRegex.checked);
                        logs.setProperty(index, "filterCase", filterCase.checked);
                        logs.setProperty(index, "filterInvert", filterInvert.checked);
                        logs.setProperty(index, "filterNumericJson", JSON.stringify(page.numericFilterCondition()));
                        logs.setProperty(index, "searchText", searchInput.text);
                        logs.setProperty(index, "searchRegex", searchRegex.checked);
                        logs.setProperty(index, "searchCase", searchCase.checked);
                        logs.setProperty(index, "follow", follow.checked);
                        root.scheduleSave();
                    }
                    Component.onCompleted: {
                        viewReady = true;
                        backend.set_view_active(viewActive);
                    }

                    LogBackend {
                        id: backend
                        Component.onCompleted: {
                            set_view_active(page.viewActive);
                            set_syslog_badges(page.source === "syslog-udp" && page.syslogBadges);
                            set_encoding(page.encoding);
                            if (page.remote)
                                connect_profile(root.profileById(page.connectionId), page.logPath, page.initial, page.capacity, page.elevation, page.runUser, page.source);
                            else if (page.source === "custom")
                                run_local(page.logPath, page.capacity);
                            else if (page.source === "syslog-udp")
                                listen_syslog(page.logPath, page.capacity);
                            else
                                open(page.logPath, page.initial, page.capacity);
                            set_numeric_filter(page.filterEnabled ? page.filterText : "", page.filterRegex,
                                page.filterCase, page.filterEnabled && page.filterInvert,
                                page.filterEnabled ? page.filterNumeric : null);
                            search(page.searchText, page.searchRegex, page.searchCase);
                            set_trap_rules(page.trapRules);
                        }
                        onChanged: {
                            // qmllint disable missing-property
                            const tab = tabRepeater.itemAt(page.index);
                            if (tab) tab.connected = backend.connected;
                            // qmllint enable missing-property
                            if (authPrompt !== "" && !authDialog.visible)
                                authDialog.open();
                        }
                        onReceived: {
                            if (page.index !== tabs.currentIndex)
                                logs.setProperty(page.index, "unread", true);
                        }
                        onTrapped: {
                            if (page.index !== tabs.currentIndex) {
                                logs.setProperty(page.index, "unread", true);
                                logs.setProperty(page.index, "alert", true);
                            }
                        }
                    }
                    Component.onDestruction: backend.stop()
                    function testAnalysisControls() {
                        analysisDialog.open();
                        testLoadAccessAnalysisSettings();
                        analysisDialog.apply();
                    }
                    function testLoadAccessAnalysisSettings() {
                        analysisDialog.loadSavedPreset("builtin-apache");
                        const s = backend.analysis_preset(false);
                        analysisRegex.text = s.regex;
                        analysisScript.text = s.script;
                        analysisQuery.text = "path, count()";
                    }
                    function testAnalysisInput(format, filter, fields, metric, operation) {
                        analysisFormat.currentIndex = format;
                        analysisFilterMode.currentIndex = 0;
                        analysisFilter.text = filter;
                        analysisExclude.checked = false;
                        analysisIgnoreCase.checked = false;
                        analysisDelimiter.text = ",";
                        analysisMappings.text = JSON.stringify(fields);
                        analysisUseRoto.checked = false;
                        analysisQuery.text = ["count()", "sum(" + metric + ")", "avg(" + metric + ")", "min(" + metric + ")", "max(" + metric + ")"][operation];
                        analysisDialog.apply();
                    }
                    function testAnalysisQuery(text) {
                        testLoadAccessAnalysisSettings();
                        analysisFilter.text = '"';
                        analysisQuery.text = text;
                        analysisDialog.apply();
                    }
                    function testSaveAnalysisPreset() {
                        analysisMergeFirstToggle.checked = true;
                        analysisDialog.rememberMergeFirstColumn();
                        analysisPresetName.text = "Integration preset";
                        if (!analysisDialog.savePreset(false)) return false;
                        const id = analysisDialog.selectedPresetId;
                        analysisQuery.text = "method, path, count()";
                        if (!analysisDialog.savePreset(true)) return false;
                        analysisDialog.loadSavedPreset("builtin-apache");
                        if (analysisMergeFirstToggle.checked) return false;
                        analysisDialog.loadSavedPreset(id);
                        if (!analysisMergeFirstToggle.checked || !page.analysisMergeFirstColumn) return false;
                        if (analysisQuery.text !== "method, path, count()" || analysisFilter.text !== '"') return false;
                        analysisPresetName.text = "Temporary preset";
                        if (!analysisDialog.savePreset(false)) return false;
                        analysisDialog.deletePreset();
                        analysisDialog.loadSavedPreset(id);
                        // Tab override differs from its preset, and must survive restart.
                        analysisMergeFirstToggle.checked = false;
                        analysisDialog.rememberMergeFirstColumn();
                        return analysisQuery.text === "method, path, count()"
                            && logs.get(page.index).analysisPresetId === id;
                    }
                    function testSelectAnalysisPreset(id) {
                        analysisDialog.loadSavedPreset(id);
                        return logs.get(page.index).analysisPresetId === id;
                    }
                    function testRestoreAnalysisPreset() {
                        const preset = workspace.analysisPresets.find(p => p.name === "Integration preset");
                        if (!preset || workspace.analysisPresets.length !== 3) return false;
                        if (workspace.analysisPresets.some(p => p.id === "builtin-access")) return false;
                        if (preset.settings.mergeFirstColumn !== true || page.analysisMergeFirstColumn) return false;
                        if (page.analysisPresetId !== preset.id) return false;
                        analysisDialog.open();
                        return analysisQuery.text === "method, path, count()"
                            && analysisFilter.text === '"' && analysisUseRoto.checked
                            && analysisFormat.currentIndex === 0 && !analysisMergeFirstToggle.checked;
                    }
                    function testRestoreOtherAnalysisPreset() {
                        if (page.analysisPresetId !== "builtin-apache") return false;
                        analysisDialog.open();
                        if (analysisDialog.selectedPresetId !== "builtin-apache") return false;
                        analysisDialog.loadSavedPreset("builtin-access");
                        return analysisDialog.selectedPresetId === "builtin-apache"
                            && logs.get(page.index).analysisPresetId === "builtin-apache";
                    }
                    Dialog {
                        id: analysisDialog
                        parent: Overlay.overlay
                        x: (parent.width - width) / 2
                        y: (parent.height - height) / 2
                        width: Math.min(900, root.width - 40)
                        height: Math.min(900, root.height - 40)
                        modal: false
                        closePolicy: Popup.CloseOnEscape
                        title: root.tr("Log analysis (Roto)")
                        standardButtons: Dialog.Close
                        readonly property real minimumResizeWidth: Math.min(640, parent.width)
                        readonly property real minimumResizeHeight: Math.min(360, parent.height)
                        MouseArea {
                            id: analysisResizeHandle
                            parent: analysisDialog.footer
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            width: 20
                            height: 20
                            z: 100
                            cursorShape: Qt.SizeFDiagCursor
                            property point pressPosition
                            property real startWidth: 0
                            property real startHeight: 0
                            onPressed: mouse => {
                                pressPosition = mapToItem(analysisDialog.parent, mouse.x, mouse.y);
                                startWidth = analysisDialog.width;
                                startHeight = analysisDialog.height;
                                // Resizing must not recenter a dialog that has not been moved yet.
                                analysisDialog.x = analysisDialog.x;
                                analysisDialog.y = analysisDialog.y;
                            }
                            onPositionChanged: mouse => {
                                if (!pressed) return;
                                const position = mapToItem(analysisDialog.parent, mouse.x, mouse.y);
                                analysisDialog.width = Math.max(analysisDialog.minimumResizeWidth,
                                    Math.min(startWidth + position.x - pressPosition.x, analysisDialog.parent.width));
                                analysisDialog.height = Math.max(analysisDialog.minimumResizeHeight,
                                    Math.min(startHeight + position.y - pressPosition.y, analysisDialog.parent.height));
                            }
                            Repeater {
                                model: 3
                                delegate: Rectangle {
                                    required property int index
                                    width: 3 + index * 4
                                    height: 1
                                    rotation: -45
                                    anchors.right: parent ? parent.right : undefined
                                    anchors.bottom: parent ? parent.bottom : undefined
                                    anchors.rightMargin: 3
                                    anchors.bottomMargin: 4 + index * 3
                                    color: analysisDialog.palette.mid
                                }
                            }
                        }
                        header: Label {
                            id: analysisTitleBar
                            text: analysisDialog.title
                            font.bold: true
                            padding: 12
                            elide: Text.ElideRight
                            background: Rectangle { color: analysisDialog.palette.button }
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: pressed ? Qt.ClosedHandCursor : Qt.OpenHandCursor
                                property point pressPosition
                                property real startX: 0
                                property real startY: 0
                                onPressed: mouse => {
                                    pressPosition = mapToItem(analysisDialog.parent, mouse.x, mouse.y);
                                    startX = analysisDialog.x;
                                    startY = analysisDialog.y;
                                }
                                onPositionChanged: mouse => {
                                    if (!pressed) return;
                                    const position = mapToItem(analysisDialog.parent, mouse.x, mouse.y);
                                    analysisDialog.x = Math.max(80 - analysisDialog.width,
                                        Math.min(startX + position.x - pressPosition.x, analysisDialog.parent.width - 80));
                                    analysisDialog.y = Math.max(0,
                                        Math.min(startY + position.y - pressPosition.y, analysisDialog.parent.height - analysisTitleBar.height));
                                }
                            }
                        }
                        property bool initialized: false
                        property string settingsError: ""
                        property string selectedPresetId: "builtin-apache"
                        function rememberPreset() {
                            logs.setProperty(page.index, "analysisPresetId", selectedPresetId);
                            root.scheduleSave();
                        }
                        onSelectedPresetIdChanged: rememberPreset()
                        readonly property var presetRows: workspace.analysisPresets
                        readonly property var selectedPreset: presetRows.find(p => p.id === selectedPresetId)
                        readonly property var statistics: backend.analysisResult
                        function rememberMergeFirstColumn() {
                            logs.setProperty(page.index, "analysisMergeFirstColumn", analysisMergeFirstToggle.checked);
                            root.scheduleSave();
                        }
                        function loadSavedPreset(id) {
                            const preset = presetRows.find(p => p.id === id)
                                || presetRows.find(p => p.id === "builtin-apache");
                            if (!preset) return;
                            const s = preset.settings;
                            selectedPresetId = preset.id;
                            rememberPreset();
                            analysisPresetName.text = preset.builtin ? "" : preset.name;
                            analysisRegex.text = s.regex;
                            analysisScript.text = s.script;
                            analysisQuery.text = s.query;
                            analysisFormat.currentIndex = ["regex", "delimiter", "whitespace", "json"].indexOf(s.format);
                            analysisDelimiter.text = s.delimiter === "\t" ? "\\t" : s.delimiter;
                            analysisFilter.text = s.filter;
                            analysisFilterMode.currentIndex = ["contains", "prefix", "regex"].indexOf(s.filterMode);
                            analysisMappings.text = JSON.stringify(s.fields, null, 2);
                            analysisUseRoto.checked = s.useRoto;
                            analysisExclude.checked = s.exclude;
                            analysisIgnoreCase.checked = s.ignoreCase;
                            analysisMergeFirstToggle.checked = s.mergeFirstColumn === true;
                            rememberMergeFirstColumn();
                            settingsError = "";
                        }
                        function editorSettings() {
                            let fields;
                            try {
                                fields = JSON.parse(analysisMappings.text);
                                if (!Array.isArray(fields)) throw new Error("Expected an array");
                            } catch (e) {
                                settingsError = root.tr("Invalid field mappings") + ": " + e;
                                return null;
                            }
                            settingsError = "";
                            return {
                                    regex: analysisRegex.text,
                                    script: analysisScript.text,
                                    useRoto: analysisUseRoto.checked,
                                    query: analysisQuery.text,
                                    format: ["regex", "delimiter", "whitespace", "json"][analysisFormat.currentIndex],
                                    delimiter: analysisDelimiter.text === "\\t" ? "\t" : analysisDelimiter.text,
                                    filter: analysisFilter.text,
                                    filterMode: ["contains", "prefix", "regex"][analysisFilterMode.currentIndex],
                                    ignoreCase: analysisIgnoreCase.checked,
                                    exclude: analysisExclude.checked,
                                    mergeFirstColumn: analysisMergeFirstToggle.checked,
                                    fields: fields
                                };
                        }
                        function apply() {
                            const s = editorSettings();
                            if (!s) return;
                            backend.analyze_input(s.useRoto ? s.script : "", s.regex, "", "", "count", s);
                        }
                        function savePreset(overwrite) {
                            const s = editorSettings();
                            if (!s) return false;
                            const result = workspace.save_analysis_preset(overwrite ? selectedPresetId : "", analysisPresetName.text, s);
                            if (result.error) { settingsError = result.error; return false; }
                            selectedPresetId = result.id;
                            analysisPresetName.text = selectedPreset.name;
                            return true;
                        }
                        function deletePreset() {
                            const result = workspace.delete_analysis_preset(selectedPresetId);
                            if (result.error) { settingsError = result.error; return; }
                            loadSavedPreset("builtin-apache");
                        }
                        onOpened: {
                            root.translateButtons(analysisDialog);
                            if (!initialized) {
                                const savedMerge = page.analysisMergeFirstColumn;
                                loadSavedPreset(page.analysisPresetId);
                                analysisMergeFirstToggle.checked = savedMerge;
                                rememberMergeFirstColumn();
                                initialized = true;
                            }
                            else if (!selectedPreset) loadSavedPreset("builtin-apache");
                        }
                        onRejected: close()
                        contentItem: ScrollView {
                          id: analysisContent
                          clip: true
                          contentWidth: availableWidth
                          ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                          ColumnLayout {
                            width: analysisContent.availableWidth
                            height: Math.max(implicitHeight, analysisContent.availableHeight)
                            spacing: 8
                            Label {
                                text: root.tr("Analyze retained logs, independently of the display filter. Updates every 2 seconds after completion.")
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                            RowLayout {
                                Label { text: root.tr("Analysis preset") }
                                ComboBox {
                                    Layout.fillWidth: true
                                    model: analysisDialog.presetRows.map(p => p.name + (p.builtin ? " [" + root.tr("Default") + "]" : ""))
                                    currentIndex: analysisDialog.presetRows.findIndex(p => p.id === analysisDialog.selectedPresetId)
                                    onActivated: analysisDialog.loadSavedPreset(analysisDialog.presetRows[currentIndex].id)
                                }
                                Item { Layout.fillWidth: true }
                                Button {
                                    id: analysisApplyButton
                                    text: root.tr("Apply / recalculate")
                                    palette.button: "#1D4F1D"
                                    palette.buttonText: "#ffffff"
                                    onClicked: analysisDialog.apply()
                                }
                                Button {
                                    id: analysisStopButton
                                    text: root.tr("Stop analysis")
                                    palette.button: "#4F1916"
                                    palette.buttonText: "#ffffff"
                                    onClicked: backend.stop_analysis()
                                }
                            }
                            RowLayout {
                                TextField { id: analysisPresetName; Layout.fillWidth: true; placeholderText: root.tr("Preset name") }
                                Button { text: root.tr("Save as new preset"); onClicked: analysisDialog.savePreset(false) }
                                Button { text: root.tr("Overwrite preset"); enabled: !!analysisDialog.selectedPreset && !analysisDialog.selectedPreset.builtin; onClicked: analysisDialog.savePreset(true) }
                                Button { text: root.tr("Delete preset"); enabled: !!analysisDialog.selectedPreset && !analysisDialog.selectedPreset.builtin; onClicked: analysisPresetDelete.open() }
                            }
                            RowLayout {
                                Label { text: root.tr("Pre-extraction line filter") }
                                ComboBox { id: analysisFilterMode; model: [root.tr("Contains"), root.tr("Starts with"), root.tr("Regex")] }
                                TextField { id: analysisFilter; Layout.fillWidth: true; placeholderText: root.tr("Empty = all") }
                                CheckBox { id: analysisExclude; text: root.tr("Exclude matches") }
                                CheckBox { id: analysisIgnoreCase; text: root.tr("Ignore case") }
                            }
                            ToolButton {
                                id: analysisExtractionToggle
                                checkable: true
                                text: (checked ? "▼ " : "▶ ") + root.tr("Extraction format")
                                Accessible.name: text
                            }
                            ColumnLayout {
                                id: analysisExtractionSection
                                visible: analysisExtractionToggle.checked
                                Layout.fillWidth: true
                                spacing: 8
                                RowLayout {
                                    ComboBox { id: analysisFormat; model: [root.tr("Regex"), root.tr("Delimiter"), root.tr("Whitespace"), "JSON Lines"] }
                                    TextField { id: analysisDelimiter; visible: analysisFormat.currentIndex === 1; text: ","; placeholderText: root.tr("Delimiter (tab: \\t)"); Layout.fillWidth: true }
                                }
                                Label { visible: analysisFormat.currentIndex === 0; text: root.tr("Extraction regex (named captures)") }
                                TextField { id: analysisRegex; visible: analysisFormat.currentIndex === 0; Layout.fillWidth: true; selectByMouse: true }
                                Label { text: root.tr("Field mappings (JSON array; columns start at 0)"); wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                ScrollView {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 75
                                    TextArea {
                                        id: analysisMappings; text: "[]"; font.family: "monospace"; selectByMouse: true; textFormat: TextEdit.PlainText
                                        placeholderText: '[{"name":"total","source":"total","type":"number","required":true}]'
                                    }
                                }
                                Label {
                                    text: root.tr("Source: capture name, column index, or JSON Pointer. Empty mappings preserve regex captures.")
                                    wrapMode: Text.WordWrap; Layout.fillWidth: true
                                }
                            }
                            Label { visible: analysisDialog.settingsError !== ""; text: analysisDialog.settingsError; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                            RowLayout {
                                Layout.fillWidth: true
                                ToolButton {
                                    id: analysisRotoToggle
                                    checkable: true
                                    text: (checked ? "▼ " : "▶ ") + root.tr("Enable Roto transformation")
                                    Accessible.name: text
                                }
                                Item { Layout.fillWidth: true }
                                CheckBox {
                                    id: analysisUseRoto
                                    checked: true
                                    Accessible.name: root.tr("Enable Roto transformation")
                                }
                            }
                            ScrollView {
                                id: analysisRotoSection
                                visible: analysisRotoToggle.checked && analysisUseRoto.checked
                                Layout.fillWidth: true
                                Layout.preferredHeight: 130
                                TextArea {
                                    id: analysisScript
                                    font.family: "monospace"
                                    wrapMode: TextEdit.NoWrap
                                    selectByMouse: true
                                    textFormat: TextEdit.PlainText
                                }
                            }
                            ToolButton {
                                id: analysisPreviewToggle
                                checkable: true
                                text: (checked ? "▼ " : "▶ ") + root.tr("Input preview (first 5 lines; after applying)")
                                Accessible.name: text
                            }
                            ScrollView {
                                id: analysisPreviewSection
                                visible: analysisPreviewToggle.checked
                                Layout.fillWidth: true; Layout.preferredHeight: 100
                                TextArea {
                                    readOnly: true; selectByMouse: true; font.family: "monospace"; textFormat: TextEdit.PlainText
                                    text: JSON.stringify(analysisDialog.statistics.preview || [], null, 2)
                                }
                            }
                            RowLayout {
                                Label { text: root.tr("Result columns") }
                                TextField { id: analysisQuery; Layout.fillWidth: true; text: "count()"; selectByMouse: true; placeholderText: "path, method, status, avg(elapsed), max(elapsed), count()" }
                            }
                            Label { text: root.tr("Plain fields group rows; aggregates: avg, max, min, sum, count()."); wrapMode: Text.WordWrap; Layout.fillWidth: true }
                            Label {
                                Layout.fillWidth: true
                                wrapMode: Text.WordWrap
                                text: analysisDialog.statistics.total !== undefined
                                    ? root.tr("Retained log statistics") + ": " + analysisDialog.statistics.total
                                        + " | " + root.tr("Accepted") + ": " + analysisDialog.statistics.matched
                                        + " | " + root.tr("Skipped") + ": " + analysisDialog.statistics.skipped
                                        + " | " + root.tr("Unmatched") + ": " + analysisDialog.statistics.unmatched
                                        + " | " + root.tr("Errors") + ": " + analysisDialog.statistics.errors
                                        + " | " + Number(analysisDialog.statistics.elapsedMs).toFixed(1) + " ms"
                                        + " (JIT: " + Number(analysisDialog.statistics.compileMs).toFixed(1) + " ms)"
                                        + " | " + root.tr("Lines") + ": " + (analysisDialog.statistics.firstLine ?? "—") + "–" + (analysisDialog.statistics.lastLine ?? "—")
                                    : ""
                            }
                            RowLayout {
                                id: analysisResultToolbar
                                Layout.fillWidth: true
                                CheckBox {
                                    id: analysisTextToggle
                                    text: root.tr("Text display")
                                }
                                CheckBox {
                                    id: analysisMergeFirstToggle
                                    text: root.tr("Merge repeated first-column values")
                                    enabled: !analysisTextToggle.checked
                                    checked: page.analysisMergeFirstColumn
                                    onToggled: analysisDialog.rememberMergeFirstColumn()
                                }
                                Item { Layout.fillWidth: true }
                                // Reserve the slot even while idle so neither toolbar nor results jump.
                                Item {
                                    Layout.minimumWidth: 24
                                    Layout.maximumWidth: 24
                                    Layout.minimumHeight: 24
                                    Layout.maximumHeight: 24
                                    BusyIndicator {
                                        id: analysisResultBusy
                                        anchors.fill: parent
                                        running: backend.analysisBusy
                                        visible: running
                                    }
                                }
                            }
                            ColumnLayout {
                                id: analysisResultSection
                                visible: !analysisTextToggle.checked
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                Layout.minimumHeight: 80
                                Layout.preferredHeight: 180
                                spacing: 0
                                readonly property var columns: analysisDialog.statistics.columns || []
                                property string sortColumn: ""
                                property bool sortDescending: false
                                function sortBy(column) {
                                    if (sortColumn !== column) {
                                        sortDescending = false;
                                        sortColumn = column;
                                    } else if (!sortDescending) {
                                        sortDescending = true;
                                    } else {
                                        sortColumn = "";
                                        sortDescending = false;
                                    }
                                }
                                readonly property var sortedRows: {
                                    const rows = analysisDialog.statistics.rows || [];
                                    const column = columns.indexOf(sortColumn);
                                    if (column < 0) return rows;
                                    const direction = sortDescending ? -1 : 1;
                                    // Sort a copy, using the source index to keep equal values stable.
                                    return rows.map((row, index) => ({row: row, index: index})).sort((a, b) => {
                                        const left = (a.row.cells || [])[column];
                                        const right = (b.row.cells || [])[column];
                                        // Missing values remain last in either direction.
                                        if (left == null || right == null) {
                                            if (left != null) return -1;
                                            if (right != null) return 1;
                                            return a.index - b.index;
                                        }
                                        const comparison = typeof left === "number" && typeof right === "number"
                                            ? (left < right ? -1 : left > right ? 1 : 0)
                                            : String(left).localeCompare(String(right));
                                        return comparison * direction || a.index - b.index;
                                    }).map(item => item.row);
                                }
                                // Precompute run starts once per refresh/sort, not once per cell.
                                readonly property var firstColumnRunStarts: {
                                    const starts = [];
                                    let start = 0;
                                    for (let i = 0; i < sortedRows.length; ++i) {
                                        const value = (sortedRows[i].cells || [])[0];
                                        const previous = i > 0 ? (sortedRows[i - 1].cells || [])[0] : undefined;
                                        if (i === 0 || value == null || value !== previous) start = i;
                                        starts.push(start);
                                    }
                                    return starts;
                                }
                                property var customColumnWidths: ({})
                                readonly property real minimumColumnWidth: 64
                                function resizeColumn(name, width) {
                                    const updated = Object.assign({}, customColumnWidths);
                                    Object.defineProperty(updated, name, {
                                        value: Math.max(minimumColumnWidth, width),
                                        enumerable: true, configurable: true, writable: true
                                    });
                                    customColumnWidths = updated;
                                }
                                readonly property var widths: columns.map(name =>
                                    Object.prototype.hasOwnProperty.call(customColumnWidths, name)
                                        ? customColumnWidths[name]
                                        : Math.max(180, Math.min(300, analysisCellMetrics.advanceWidth(name) + 28)))
                                readonly property real tableWidth: widths.reduce((sum, width) => sum + width, 0)
                                FontMetrics { id: analysisCellMetrics; font.family: "monospace" }
                                Flickable {
                                    id: analysisHeader
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 32
                                    clip: true
                                    interactive: false
                                    contentWidth: analysisResultSection.tableWidth
                                    contentX: analysisTable.contentX
                                    Row {
                                        Repeater {
                                            model: analysisResultSection.columns
                                            delegate: Rectangle {
                                                id: analysisColumnHeader
                                                required property int index
                                                required property string modelData
                                                objectName: "analysisHeader_" + index
                                                width: analysisResultSection.widths[index]
                                                height: 32
                                                color: palette.button
                                                border.color: palette.mid
                                                Label {
                                                    anchors.fill: parent
                                                    anchors.margins: 8
                                                    text: modelData + (analysisResultSection.sortColumn === modelData
                                                        ? (analysisResultSection.sortDescending ? " ▼" : " ▲") : "")
                                                    textFormat: Text.PlainText
                                                    font.bold: true
                                                    elide: Text.ElideRight
                                                    verticalAlignment: Text.AlignVCenter
                                                }
                                                HoverHandler { id: headerHover }
                                                MouseArea {
                                                    anchors.fill: parent
                                                    cursorShape: Qt.PointingHandCursor
                                                    onClicked: analysisResultSection.sortBy(modelData)
                                                }
                                                MouseArea {
                                                    objectName: "analysisColumnResize_" + analysisColumnHeader.index
                                                    anchors.right: parent.right
                                                    anchors.top: parent.top
                                                    anchors.bottom: parent.bottom
                                                    width: 16
                                                    z: 1
                                                    cursorShape: Qt.SizeHorCursor
                                                    preventStealing: true
                                                    property real pressX: 0
                                                    property real startWidth: 0
                                                    onPressed: mouse => {
                                                        pressX = mapToItem(analysisHeader, mouse.x, mouse.y).x;
                                                        startWidth = analysisColumnHeader.width;
                                                    }
                                                    onPositionChanged: mouse => {
                                                        if (!pressed) return;
                                                        const x = mapToItem(analysisHeader, mouse.x, mouse.y).x;
                                                        analysisResultSection.resizeColumn(analysisColumnHeader.modelData,
                                                            startWidth + x - pressX);
                                                    }
                                                }
                                                ToolTip {
                                                    id: headerTooltip
                                                    visible: headerHover.hovered
                                                    delay: 500
                                                    text: modelData
                                                    width: Math.min(600, implicitWidth)
                                                    contentItem: Label { text: headerTooltip.text; textFormat: Text.PlainText; wrapMode: Text.WrapAnywhere }
                                                }
                                            }
                                        }
                                    }
                                }
                                ScrollView {
                                    id: analysisResults
                                    clip: true
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    ScrollBar.vertical.policy: ScrollBar.AlwaysOn
                                    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                                    ListView {
                                        id: analysisTable
                                        clip: true
                                        contentWidth: Math.max(width, analysisResultSection.tableWidth)
                                        onContentWidthChanged: Qt.callLater(() => {
                                            contentX = Math.max(originX, Math.min(contentX,
                                                originX + Math.max(0, contentWidth - width)));
                                        })
                                        flickableDirection: Flickable.HorizontalAndVerticalFlick
                                        boundsBehavior: Flickable.StopAtBounds
                                        readonly property var resultRows: analysisResultSection.sortedRows
                                        property bool restoringPosition: false
                                        property real savedX: 0
                                        property real savedY: 0
                                        function updateRows() {
                                            if (!restoringPosition) {
                                                savedX = contentX;
                                                savedY = contentY;
                                            }
                                            restoringPosition = true;
                                            model = resultRows;
                                            Qt.callLater(() => {
                                                forceLayout();
                                                contentX = Math.max(originX, Math.min(savedX,
                                                    originX + Math.max(0, contentWidth - width)));
                                                contentY = Math.max(originY, Math.min(savedY,
                                                    originY + Math.max(0, contentHeight - height)));
                                                restoringPosition = false;
                                            });
                                        }
                                        onResultRowsChanged: updateRows()
                                        Component.onCompleted: updateRows()
                                        delegate: Row {
                                            id: analysisRow
                                            required property int index
                                            required property var modelData
                                            height: 30
                                            Repeater {
                                                model: analysisResultSection.columns.length
                                                delegate: Rectangle {
                                                    id: analysisCell
                                                    required property int index
                                                    readonly property var value: (analysisRow.modelData.cells || [])[index]
                                                    readonly property string fullText: String(value ?? "—")
                                                    readonly property bool mergedFirstColumn: index === 0 && analysisMergeFirstToggle.checked
                                                    readonly property int runStart: analysisResultSection.firstColumnRunStarts[analysisRow.index] ?? analysisRow.index
                                                    readonly property bool continuation: mergedFirstColumn && runStart !== analysisRow.index
                                                    readonly property bool runEnd: analysisRow.index + 1 >= analysisResultSection.sortedRows.length
                                                        || analysisResultSection.firstColumnRunStarts[analysisRow.index + 1] !== runStart
                                                    objectName: "analysisCell_" + analysisRow.index + "_" + index
                                                    width: analysisResultSection.widths[index]
                                                    height: 30
                                                    color: (mergedFirstColumn ? runStart : (analysisRow ? analysisRow.index : 0)) % 2 ? palette.alternateBase : palette.base
                                                    border.width: mergedFirstColumn ? 0 : 1
                                                    border.color: palette.midlight
                                                    Rectangle {
                                                        width: 1; height: parent.height
                                                        color: palette.midlight
                                                        visible: analysisCell.mergedFirstColumn
                                                    }
                                                    Rectangle {
                                                        anchors.right: parent.right
                                                        width: 1; height: parent.height
                                                        color: palette.midlight
                                                        visible: analysisCell.mergedFirstColumn
                                                    }
                                                    Rectangle {
                                                        objectName: "analysisCellTopBorder"
                                                        width: parent.width; height: 1
                                                        color: palette.midlight
                                                        visible: analysisCell.mergedFirstColumn && !analysisCell.continuation
                                                    }
                                                    Rectangle {
                                                        objectName: "analysisCellBottomBorder"
                                                        anchors.bottom: parent.bottom
                                                        width: parent.width; height: 1
                                                        color: palette.midlight
                                                        visible: analysisCell.mergedFirstColumn && analysisCell.runEnd
                                                    }
                                                    Label {
                                                        id: cellLabel
                                                        objectName: "analysisCellLabel"
                                                        visible: !analysisCell.continuation
                                                        anchors.fill: parent
                                                        anchors.margins: 7
                                                        text: typeof analysisCell.value === "number" && !Number.isInteger(analysisCell.value)
                                                            ? analysisCell.value.toFixed(3) : analysisCell.fullText
                                                        textFormat: Text.PlainText
                                                        font.family: "monospace"
                                                        elide: Text.ElideRight
                                                        verticalAlignment: Text.AlignVCenter
                                                        horizontalAlignment: typeof analysisCell.value === "number" ? Text.AlignRight : Text.AlignLeft
                                                    }
                                                    HoverHandler { id: cellHover }
                                                    ToolTip {
                                                        id: cellTooltip
                                                        objectName: "analysisCellTooltip"
                                                        visible: cellHover.hovered
                                                        delay: 500
                                                        text: analysisCell.fullText
                                                        width: Math.min(600, implicitWidth)
                                                        contentItem: Label { text: cellTooltip.text; textFormat: Text.PlainText; wrapMode: Text.WrapAnywhere }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                ScrollBar {
                                    id: analysisHorizontalScroll
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: implicitHeight
                                    orientation: Qt.Horizontal
                                    visible: analysisTable.contentWidth > analysisTable.width
                                    policy: ScrollBar.AlwaysOn
                                    size: analysisTable.visibleArea.widthRatio
                                    position: analysisTable.visibleArea.xPosition
                                    onPositionChanged: {
                                        if (pressed)
                                            analysisTable.contentX = position * analysisTable.contentWidth;
                                    }
                                }
                            }
                            ScrollView {
                                id: analysisTextResults
                                visible: analysisTextToggle.checked
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                Layout.minimumHeight: 80
                                Layout.preferredHeight: 180
                                clip: true
                                ScrollBar.vertical.policy: ScrollBar.AlwaysOn
                                TextArea {
                                    id: analysisResultText
                                    readOnly: true
                                    selectByMouse: true
                                    font.family: "monospace"
                                    wrapMode: TextEdit.NoWrap
                                    textFormat: TextEdit.PlainText
                                    property bool restoringPosition: false
                                    property real savedX: 0
                                    property real savedY: 0
                                    readonly property string resultText: {
                                        const r = analysisDialog.statistics;
                                        const display = value => typeof value === "string" ? JSON.stringify(value) : String(value ?? "—");
                                        return (r.columns || []).join("\t") + "\n"
                                            + analysisResultSection.sortedRows.map(row => (row.cells || []).map(display).join("\t")).join("\n");
                                    }
                                    function updateResultText() {
                                        if (text === resultText) return;
                                        const view = analysisTextResults.contentItem;
                                        if (!restoringPosition) {
                                            savedX = view.contentX;
                                            savedY = view.contentY;
                                        }
                                        restoringPosition = true;
                                        text = resultText;
                                        Qt.callLater(() => {
                                            view.contentX = Math.max(view.originX, Math.min(savedX,
                                                view.originX + Math.max(0, view.contentWidth - view.width)));
                                            view.contentY = Math.max(view.originY, Math.min(savedY,
                                                view.originY + Math.max(0, view.contentHeight - view.height)));
                                            restoringPosition = false;
                                        });
                                    }
                                    onResultTextChanged: updateResultText()
                                    Component.onCompleted: updateResultText()
                                }
                            }
                            ScrollView {
                                id: analysisDiagnostics
                                visible: !!analysisDialog.statistics.error || (analysisDialog.statistics.samples || []).length > 0
                                Layout.fillWidth: true
                                Layout.preferredHeight: 110
                                clip: true
                                TextArea {
                                    id: analysisDiagnosticText
                                    readOnly: true
                                    selectByMouse: true
                                    font.family: "monospace"
                                    wrapMode: TextEdit.WrapAnywhere
                                    textFormat: TextEdit.PlainText
                                    text: analysisDialog.statistics.error || (root.tr("Diagnostic samples (up to 5; log text shortened)") + "\n"
                                        + (analysisDialog.statistics.samples || []).map(s => "#" + s.line + " " + s.reason + "\n" + s.text).join("\n\n"))
                                }
                            }
                        }
                      }
                        Dialog {
                            id: analysisPresetDelete
                            parent: Overlay.overlay
                            anchors.centerIn: parent
                            width: Math.min(420, root.width - 40)
                            modal: true
                            title: root.tr("Delete preset?")
                            standardButtons: Dialog.Ok | Dialog.Cancel
                            onOpened: root.translateButtons(analysisPresetDelete)
                            onAccepted: analysisDialog.deletePreset()
                            contentItem: Label { text: analysisDialog.selectedPreset ? analysisDialog.selectedPreset.name : ""; wrapMode: Text.WordWrap }
                        }
                    }
                    Menu {
                        id: selectionMenu
                        property string selectedText: ""
                        property var textItem: null
                        MenuItem {
                            text: root.tr("Copy")
                            enabled: selectionMenu.selectedText !== ""
                            onTriggered: { if (selectionMenu.textItem) selectionMenu.textItem.copy(); }
                        }
                        MenuItem {
                            text: root.tr("Add to Text Trap…")
                            enabled: selectionMenu.selectedText !== "" && page.trapRules.length < 10
                            onTriggered: page.editTrap(-1, selectionMenu.selectedText)
                        }
                    }
                    Dialog {
                        id: trapEditor
                        property int tagIndex: -1
                        property string error: ""
                        parent: Overlay.overlay
                        anchors.centerIn: parent
                        width: Math.min(560, root.width - 40)
                        modal: true
                        title: tagIndex >= 0 ? root.tr("Edit text trap") : root.tr("Add text trap")
                        onOpened: editTrapText.forceActiveFocus()
                        function save() {
                            const rules = page.trapRules.slice();
                            const rule = {text: editTrapText.text, enabled: editTrapEnabled.checked, regex: editTrapRegex.checked,
                                ignoreCase: editTrapCase.checked, color: editTrapColor.text,
                                numeric:editTrapNumeric.checked ? {capture:editTrapCapture.value, operator:editTrapOperator.currentText, value:editTrapValue.text} : null,
                                background:editTrapBackground.checked, opacity:editTrapOpacity.value, scope:editTrapScope.codes[editTrapScope.currentIndex]};
                            if (tagIndex >= 0) rules[tagIndex] = rule;
                            else rules.push(rule);
                            error = backend.validate_trap_rules(rules);
                            if (error === "" && page.commitTrapRules(rules)) close();
                        }
                        contentItem: ColumnLayout {
                            Label { text: root.tr("Text or regular expression to add") }
                            TextField {
                                id: editTrapText
                                maximumLength: 1024
                                Layout.fillWidth: true
                                onAccepted: trapEditor.save()
                            }
                            RowLayout {
                                CheckBox { id: editTrapEnabled; text: root.tr("Enable"); checked: true }
                                CheckBox { id: editTrapRegex; text: root.tr("Regex") }
                                CheckBox { id: editTrapCase; text: root.tr("Ignore case") }
                            }
                            CheckBox { id: editTrapNumeric; text: root.tr("Numeric condition"); onToggled: if (checked) editTrapRegex.checked = true }
                            RowLayout {
                                visible: editTrapNumeric.checked
                                Label { text: root.tr("Capture group") }
                                SpinBox { id: editTrapCapture; from: 1; to: 99; value: 1; editable: true }
                                ComboBox { id: editTrapOperator; model: [">", ">=", "<", "<=", "==", "!="] }
                                TextField { id: editTrapValue; text: "100"; maximumLength: 128; placeholderText: root.tr("Comparison value"); Layout.fillWidth: true }
                            }
                            RowLayout {
                                CheckBox { id: editTrapBackground; text: root.tr("Background highlight") }
                                Label { text: root.tr("Opacity (%)"); visible: editTrapBackground.checked }
                                SpinBox { id: editTrapOpacity; visible: editTrapBackground.checked; from: 0; to: 100; value: 25; editable: true }
                            }
                            RowLayout {
                                Label { text: root.tr("Highlight scope") }
                                ComboBox {
                                    id: editTrapScope
                                    property var codes: ["match", "capture", "line"]
                                    model: [root.tr("Regex match"), root.tr("Captured number"), root.tr("Entire line")]
                                    Layout.fillWidth: true
                                }
                            }
                            Label { text: root.tr("Highlight text color") }
                            RowLayout {
                                TextField {
                                    id: editTrapColor
                                    placeholderText: "#RRGGBB"
                                    maximumLength: 7
                                    Layout.fillWidth: true
                                }
                                Rectangle {
                                    width: 28; height: 28
                                    color: /^#[0-9a-fA-F]{6}$/.test(editTrapColor.text) ? editTrapColor.text : "transparent"
                                    border.color: palette.text
                                }
                                Button {
                                    text: root.tr("Choose color…")
                                    onClicked: {
                                        if (/^#[0-9a-fA-F]{6}$/.test(editTrapColor.text)) trapColorDialog.selectedColor = editTrapColor.text;
                                        trapColorDialog.open();
                                    }
                                }
                            }
                            Row {
                                spacing: 8
                                Repeater {
                                    model: ["#35c66b", "#ef5350", "#ff9800", "#d4b000", "#42a5f5", "#ab47bc"]
                                    delegate: Button {
                                        required property string modelData
                                        width: 32; height: 28
                                        Accessible.name: modelData
                                        background: Rectangle { color: parent.modelData; radius: 3 }
                                        onClicked: editTrapColor.text = modelData
                                    }
                                }
                            }
                            Label {
                                text: trapEditor.error
                                visible: text !== ""
                                color: "red"
                                wrapMode: Text.WrapAnywhere
                                Layout.fillWidth: true
                            }
                        }
                        footer: DialogButtonBox {
                            Button {
                                text: trapEditor.tagIndex >= 0 ? root.tr("Save") : root.tr("Add")
                                DialogButtonBox.buttonRole: DialogButtonBox.ActionRole
                                onClicked: trapEditor.save()
                            }
                            Button {
                                text: root.tr("Cancel")
                                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                            }
                            onRejected: trapEditor.close()
                        }
                    }
                    ColorDialog {
                        id: trapColorDialog
                        title: root.tr("Highlight text color")
                        onAccepted: editTrapColor.text = selectedColor.toString()
                    }
                    Dialog {
                        id: authDialog
                        parent: Overlay.overlay
                        anchors.centerIn: parent
                        width: Math.min(600, root.width - 40)
                        modal: true
                        title: backend.authConfirmation ? root.tr("Verify SSH host key") : root.tr("SSH authentication / privilege escalation")
                        standardButtons: Dialog.Ok | Dialog.Cancel
                        closePolicy: Popup.NoAutoClose
                        onOpened: {
                            root.translateButtons(authDialog);
                            secret.clear();
                            rememberSecret.checked = false;
                            if (!backend.authConfirmation)
                                secret.forceActiveFocus();
                        }
                        onClosed: {
                            // A retry may arrive while the previous dialog is closing.
                            Qt.callLater(() => {
                                if (backend.authPrompt !== "" && !authDialog.visible)
                                    authDialog.open();
                            });
                        }
                        onAccepted: {
                            backend.answer_auth(backend.authConfirmation ? "yes" : secret.text, true, rememberSecret.checked);
                            secret.clear();
                        }
                        onRejected: {
                            backend.answer_auth("", false, false);
                            secret.clear();
                        }
                        ColumnLayout {
                            anchors.fill: parent
                            Label {
                                text: backend.authPrompt
                                wrapMode: Text.WrapAnywhere
                                Layout.fillWidth: true
                            }
                            Label {
                                visible: backend.authConfirmation
                                text: root.tr("Verify the fingerprint through a trusted channel before accepting.")
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                            TextField {
                                id: secret
                                visible: !backend.authConfirmation
                                echoMode: TextInput.Password
                                Layout.fillWidth: true
                                placeholderText: root.tr("Password / passphrase")
                            }
                            CheckBox {
                                id: rememberSecret
                                visible: backend.authStorable
                                text: root.tr("Save in OS credential store")
                            }
                            Button {
                                visible: backend.authStorable
                                text: root.tr("Delete credentials saved for this prompt")
                                onClicked: backend.forget_credential()
                            }
                        }
                    }
                    Timer {
                        interval: 250
                        running: true
                        repeat: true
                        onTriggered: backend.poll()
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        RowLayout {
                            Layout.fillWidth: true
                            Label {
                                text: page.logPath
                                elide: Text.ElideMiddle
                                Layout.fillWidth: true
                            }
                            Label { text: page.encoding + " → UTF-8" }
                            CheckBox {
                                id: syslogBadgesToggle
                                visible: page.source === "syslog-udp"
                                text: root.tr("Syslog labels")
                                checked: page.syslogBadges
                                onToggled: {
                                    logs.setProperty(page.index, "syslogBadges", checked);
                                    backend.set_syslog_badges(checked);
                                    root.scheduleSave();
                                }
                            }
                            Button {
                                text: root.tr("Analyze…")
                                onClicked: analysisDialog.open()
                            }
                            Button {
                                id: follow
                                text: root.tr("Auto-scroll")
                                checkable: true
                                checked: page.follow
                                contentItem: Label {
                                    text: follow.text
                                    font: follow.font
                                    color: "white"
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                }
                                background: Rectangle {
                                    implicitWidth: 100
                                    implicitHeight: 28
                                    radius: 3
                                    color: follow.checked ? "#0078d4" : "#300078d4"
                                    border.color: follow.activeFocus ? follow.palette.highlight : color
                                }
                                onCheckedChanged: {
                                    page.saveView();
                                    if (checked) {
                                        scroll.displayFrozen = false;
                                        if (page.logText) page.logText.deselect();
                                        Qt.callLater(scroll.updateLogText);
                                        Qt.callLater(() => { if (page.viewActive && page.logText) page.logText.followEnd(); });
                                    }
                                }
                            }
                            ToolButton {
                                id: optionsToggle
                                checkable: true
                                checked: false
                                text: {
                                    const active = [];
                                    if (filterEnable.checked && filterInput.text !== "") active.push(root.tr("Filter enabled"));
                                    if (backend.results.length > 0) active.push(root.tr("Search results"));
                                    if (page.trapRules.some(rule => rule.enabled !== false)) active.push(root.tr("Text trap enabled"));
                                    return (checked ? "▼ " : "▶ ") + root.tr("Options")
                                        + (active.length > 0 ? " (" + active.join(", ") + ")" : "");
                                }
                                Accessible.name: text
                            }
                        }
                        RowLayout {
                            id: filterRow
                            CheckBox {
                                id: filterEnable
                                text: root.tr("Enable")
                                checked: page.filterEnabled
                                onToggled: { page.saveView(); filterTimer.stop(); page.applyFilter(); }
                            }
                            visible: optionsToggle.checked
                            TextField {
                                id: filterInput
                                text: page.filterText
                                placeholderText: root.tr("Filter (current buffer)")
                                Layout.fillWidth: true
                                onTextEdited: {
                                    filterTimer.restart();
                                    page.saveView();
                                }
                            }
                            CheckBox {
                                id: filterRegex
                                checked: page.filterRegex
                                text: root.tr("Regex")
                                onToggled: {
                                    filterTimer.restart();
                                    page.saveView();
                                }
                            }
                            CheckBox {
                                id: filterCase
                                checked: page.filterCase
                                text: root.tr("Ignore case")
                                onToggled: {
                                    filterTimer.restart();
                                    page.saveView();
                                }
                            }
                            CheckBox {
                                id: filterInvert
                                checked: page.filterInvert
                                text: root.tr("Invert")
                                onToggled: {
                                    filterTimer.restart();
                                    page.saveView();
                                }
                            }
                            CheckBox {
                                id: filterNumericEnabled
                                text: root.tr("Numeric condition")
                                checked: !!page.filterNumeric
                                onToggled: { if (checked) filterRegex.checked = true; page.saveView(); filterTimer.restart(); }
                            }
                            Button {
                                text: root.tr("Go live")
                                onClicked: backend.show_live()
                            }
                        }
                        Timer {
                            id: filterTimer
                            interval: 300
                            onTriggered: page.applyFilter()
                        }
                        RowLayout {
                            id: filterNumericRow
                            visible: optionsToggle.checked && filterNumericEnabled.checked
                            Label { id: filterCaptureLabel; text: root.tr("Capture group") }
                            SpinBox {
                                id: filterCapture
                                from: 1; to: 99; value: page.filterNumeric ? page.filterNumeric.capture : 1
                                editable: true
                                onValueModified: { page.saveView(); filterTimer.restart(); }
                            }
                            ComboBox {
                                id: filterOperator
                                model: [">", ">=", "<", "<=", "==", "!="]
                                currentIndex: Math.max(0, model.indexOf(page.filterNumeric ? page.filterNumeric.operator : ">"))
                                onActivated: { page.saveView(); filterTimer.restart(); }
                            }
                            TextField {
                                id: filterValue
                                text: page.filterNumeric ? page.filterNumeric.value.toString() : "100"
                                maximumLength: 128
                                placeholderText: root.tr("Comparison value")
                                Layout.fillWidth: true
                                onTextEdited: { page.saveView(); filterTimer.restart(); }
                            }
                        }
                        RowLayout {
                            id: searchRow
                            visible: optionsToggle.checked
                            TextField {
                                id: searchInput
                                text: page.searchText
                                onTextEdited: page.saveView()
                                placeholderText: root.tr("Search current buffer (independent of the filter)")
                                Layout.fillWidth: true
                                onAccepted: backend.search(text, searchRegex.checked, searchCase.checked)
                            }
                            CheckBox {
                                id: searchRegex
                                checked: page.searchRegex
                                onToggled: page.saveView()
                                text: root.tr("Regex")
                            }
                            CheckBox {
                                id: searchCase
                                checked: page.searchCase
                                onToggled: page.saveView()
                                text: root.tr("Ignore case")
                            }
                            Button {
                                text: root.tr("Search")
                                onClicked: backend.search(searchInput.text, searchRegex.checked, searchCase.checked)
                            }
                            Button {
                                text: root.tr("Clear")
                                onClicked: {
                                    searchInput.clear();
                                    page.saveView();
                                    backend.search("", false, false);
                                }
                            }
                        }
                        Label {
                            id: querySummary
                            text: backend.queryError || backend.summary
                            visible: optionsToggle.checked || backend.queryError !== ""
                            color: backend.queryError ? "red" : palette.text
                            Layout.fillWidth: true
                            wrapMode: Text.WordWrap
                        }
                        RowLayout {
                            id: trapRow
                            visible: optionsToggle.checked
                            Label {
                                text: root.tr("Text trap")
                            }
                            TextField {
                                id: trapInput
                                maximumLength: 1024
                                placeholderText: root.tr("Text or regular expression to add")
                                Layout.fillWidth: true
                                onAccepted: page.addTrap()
                            }
                            CheckBox {
                                id: trapRegex
                                text: root.tr("Regex")
                            }
                            CheckBox {
                                id: trapCase
                                text: root.tr("Ignore case")
                            }
                            Button {
                                text: root.tr("Add")
                                enabled: trapInput.text !== "" && page.trapRules.length < 10
                                onClicked: page.addTrap()
                            }
                            Label { text: page.trapRules.length + "/10" }
                        }
                        Flow {
                            visible: optionsToggle.checked && page.trapRules.length > 0
                            Layout.fillWidth: true
                            spacing: 6
                            Repeater {
                                model: page.trapRules
                                delegate: Row {
                                    id: trapTag
                                    required property var modelData
                                    required property int index
                                    Button {
                                        text: (trapTag.modelData.regex ? ".* " : "") + trapTag.modelData.text
                                        opacity: trapTag.modelData.enabled === false ? 0.45 : 1
                                        width: Math.min(implicitWidth, 220)
                                        contentItem: Label {
                                            text: parent.text
                                            color: trapTag.modelData.color
                                            elide: Text.ElideRight
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                        Accessible.name: root.tr("Edit text trap") + ": " + trapTag.modelData.text
                                        ToolTip.visible: hovered
                                        ToolTip.text: trapTag.modelData.text
                                        onClicked: page.editTrap(trapTag.index)
                                        TapHandler {
                                            acceptedButtons: Qt.RightButton
                                            onTapped: trapTagMenu.popup()
                                        }
                                    }
                                    Menu {
                                        id: trapTagMenu
                                        objectName: "trapTagMenu"
                                        MenuItem {
                                            text: root.tr("Enable")
                                            checkable: true
                                            checked: trapTag.modelData.enabled !== false
                                            onTriggered: page.setTrapEnabled(trapTag.index, trapTag.modelData.enabled === false)
                                        }
                                        MenuItem {
                                            text: root.tr("Edit text trap")
                                            onTriggered: page.editTrap(trapTag.index)
                                        }
                                    }
                                    ToolButton {
                                        text: "×"
                                        Accessible.name: root.tr("Delete text trap") + ": " + trapTag.modelData.text
                                        onClicked: page.removeTrap(trapTag.index)
                                    }
                                }
                            }
                        }
                        Label {
                            text: page.trapError
                            visible: optionsToggle.checked && text !== ""
                            color: "red"
                            wrapMode: Text.WrapAnywhere
                            Layout.fillWidth: true
                        }
                        ScrollView {
                            id: scroll
                            ScrollBar.vertical.policy: ScrollBar.AlwaysOn
                            property real savedY: 0
                            property real savedX: 0
                            property bool updatingText: false
                            property string appliedText: ""
                            property var appliedRevision: -1
                            property bool displayFrozen: false
                            property var appliedLineIds: []
                            property string appliedPlainText: ""
                            property int textUpdateSerial: 0
                            function lineOffsets(text) {
                                const offsets = [0];
                                for (let i = 0; i < text.length; ++i)
                                    if (text[i] === "\n") offsets.push(i + 1);
                                return offsets;
                            }
                            function refreshExpiredView() {
                                if (!logText || !displayFrozen || logText.selecting || logText.selectedText !== "") return false;
                                displayFrozen = false;
                                appliedRevision = -1;
                                savedY = 0;
                                updateLogText();
                                restoreLogPosition(0, savedX);
                                return true;
                            }
                            function updateLogText() {
                                if (!page.viewActive) return;
                                // setText resets Qt's selection and mouse-drag anchor. Keep
                                // this document stable until selection/copying is finished.
                                // Reception, decoding, archiving and traps continue in Rust.
                                if (!logText || logText.selecting || logText.selectionStart !== logText.selectionEnd)
                                    return;
                                const explicitChange = appliedRevision !== backend.viewRevision;
                                if (explicitChange) {
                                    appliedRevision = backend.viewRevision;
                                    displayFrozen = false;
                                }
                                if (!follow.checked && !explicitChange && displayFrozen) return;
                                if (!explicitChange && appliedText === backend.highlightedText
                                        && JSON.stringify(appliedLineIds) === JSON.stringify(backend.lineIds)) return;
                                let y = savedY;
                                const contextNavigation = explicitChange && backend.selectedPosition >= 0;
                                const x = contextNavigation ? 0 : savedX;
                                let anchor = -1;
                                let anchorY = 0;
                                let newAnchor = -1;
                                const nextIds = backend.lineIds;
                                const nextOffsets = lineOffsets(backend.displayText === undefined ? backend.text : backend.displayText);
                                if (!follow.checked && !explicitChange && appliedLineIds.length > 0) {
                                    const offsets = lineOffsets(appliedPlainText);
                                    for (let i = 0; i < appliedLineIds.length; ++i) {
                                        const rect = logText.positionToRectangle(Math.min(logText.length, offsets[i]));
                                        if (rect.y + rect.height > y) { anchor = i; anchorY = rect.y; break; }
                                    }
                                    if (anchor >= 0) {
                                        newAnchor = nextIds.indexOf(appliedLineIds[anchor]);
                                        if (newAnchor < 0) { displayFrozen = true; return; }
                                    }
                                }
                                updatingText = true;
                                const serial = ++textUpdateSerial;
                                appliedText = backend.highlightedText;
                                appliedPlainText = backend.displayText === undefined ? backend.text : backend.displayText;
                                appliedLineIds = nextIds.slice();
                                if (contextNavigation) {
                                    // A new TextArea discards both document and scene-graph
                                    // viewport caches left by a previously followed live view.
                                    contentItem.cancelFlick();
                                    contentItem.contentX = 0;
                                    contentItem.contentY = 0;
                                    logDocument.active = false;
                                    logDocument.active = true;
                                }
                                logText.text = appliedText;
                                if (contextNavigation) {
                                    const target = logText.positionToRectangle(Math.min(logText.length, backend.selectedPosition));
                                    y = Math.max(0, target.y - (contentItem.height - target.height) / 2);
                                }
                                if (newAnchor >= 0)
                                    y += logText.positionToRectangle(Math.min(logText.length, nextOffsets[newAnchor])).y - anchorY;
                                // TextArea resets the cursor to the start and ScrollView
                                // follows it during setText. Restore immediately so that
                                // position cannot reach a rendered frame or the scrollbar.
                                if (follow.checked) {
                                    logText.followEnd();
                                    // A user can pause follow before the deferred pass.
                                    y = contentItem.contentY;
                                    savedY = y;
                                } else {
                                    // Paused readers need the same synchronous restoration:
                                    // waiting for callLater exposes the cursor's jump to zero.
                                    restoreLogPosition(y, x);
                                }
                                Qt.callLater(() => {
                                    if (serial !== scroll.textUpdateSerial) return;
                                    if (!page.viewActive || !logText) {
                                        scroll.updatingText = false;
                                        return;
                                    }
                                    if (contextNavigation) {
                                        const target = logText.positionToRectangle(Math.min(logText.length, backend.selectedPosition));
                                        y = Math.max(0, target.y - (scroll.contentItem.height - target.height) / 2);
                                    }
                                    // Reconcile again after deferred layout has settled.
                                    scroll.restoreLogPosition(y, x);
                                    scroll.updatingText = false;
                                    scroll.savedY = scroll.contentItem.contentY;
                                    scroll.savedX = scroll.contentItem.contentX;
                                });
                            }
                            function restoreLogPosition(y, x) {
                                if (!logText || !contentItem) return;
                                if (follow.checked) logText.followEnd();
                                else {
                                    contentItem.contentY = Math.max(0, Math.min(y, contentItem.contentHeight - contentItem.height));
                                    contentItem.contentX = Math.max(0, Math.min(x, contentItem.contentWidth - contentItem.width));
                                }
                            }
                            Binding {
                                target: scroll.contentItem
                                property: "boundsBehavior"
                                value: Flickable.StopAtBounds
                            }
                            function pauseFollow() {
                                if (follow.checked) {
                                    follow.checked = false;
                                    page.saveView();
                                }
                            }
                            WheelHandler {
                                target: null
                                // Observe the wheel without blocking ScrollView's own handler.
                                blocking: false
                                onWheel: event => {
                                    if (scroll.refreshExpiredView()) { event.accepted = true; return; }
                                    if (event.angleDelta.y > 0 || event.pixelDelta.y > 0)
                                        scroll.pauseFollow();
                                    event.accepted = false;
                                }
                            }
                            Connections {
                                target: scroll.contentItem
                                function onContentYChanged() {
                                    const flick = scroll.contentItem;
                                    if (!scroll.updatingText) scroll.savedY = flick.contentY;
                                    const bar = scroll.ScrollBar.vertical;
                                    if ((flick.dragging || (bar && bar.pressed)) && flick.contentY < flick.contentHeight - flick.height - 2)
                                        scroll.pauseFollow();
                                    if (!scroll.updatingText && (flick.dragging || (bar && bar.pressed)))
                                        scroll.refreshExpiredView();
                                }
                                function onContentXChanged() {
                                    if (!scroll.updatingText) scroll.savedX = scroll.contentItem.contentX;
                                }
                            }
                            Connections {
                                target: backend
                                function onChanged() {
                                    scroll.updateLogText();
                                }
                            }
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Loader {
                                id: logDocument
                                // Loader does not get TextArea's direct-child ScrollView
                                // sizing. Fill the viewport while allowing long documents
                                // to retain their natural scrollable dimensions.
                                width: Math.max(scroll.availableWidth, item ? item.contentWidth + item.leftPadding + item.rightPadding : 0)
                                // Apply after layout instead of binding Loader height to
                                // its child's metrics (which Loader itself resizes).
                                function resizeDocument() {
                                    height = Math.max(scroll.availableHeight,
                                        item ? item.contentHeight + item.topPadding + item.bottomPadding : 0);
                                }
                                onLoaded: Qt.callLater(resizeDocument)
                                Connections {
                                    target: scroll
                                    function onAvailableHeightChanged() { Qt.callLater(logDocument.resizeDocument); }
                                }
                                sourceComponent: TextArea {
                                    id: logText
                                    readonly property alias selecting: selectionPointer.active
                                    text: ""
                                    readOnly: true
                                    Keys.onPressed: event => {
                                        if ([Qt.Key_Up, Qt.Key_Down, Qt.Key_PageUp, Qt.Key_PageDown, Qt.Key_Home, Qt.Key_End].indexOf(event.key) >= 0
                                                && scroll.refreshExpiredView()) { event.accepted = true; return; }
                                        if (event.key === Qt.Key_Up || event.key === Qt.Key_PageUp || event.key === Qt.Key_Home)
                                            scroll.pauseFollow();
                                        event.accepted = false;
                                    }
                                    textFormat: TextEdit.RichText
                                    wrapMode: TextEdit.NoWrap
                                    font.family: "monospace"
                                    selectByMouse: true
                                    onContentHeightChanged: Qt.callLater(logDocument.resizeDocument)
                                    PointHandler {
                                        id: selectionPointer
                                        target: null
                                        acceptedButtons: Qt.LeftButton
                                        onActiveChanged: {
                                            if (active) scroll.pauseFollow();
                                            else Qt.callLater(scroll.updateLogText);
                                        }
                                    }
                                    onSelectionStartChanged: Qt.callLater(scroll.updateLogText)
                                    onSelectionEndChanged: Qt.callLater(scroll.updateLogText)
                                    TapHandler {
                                        acceptedButtons: Qt.RightButton
                                        onTapped: {
                                            selectionMenu.selectedText = logText.selectedText;
                                            selectionMenu.textItem = logText;
                                            selectionMenu.popup();
                                        }
                                    }
                                    background: Rectangle {
                                        color: palette.base
                                    }
                                    // TextArea's background is viewport-pinned inside ScrollView.
                                    // A document child scrolls with the text; keep it between the
                                    // base background (z = -1) and the rendered glyphs (z = 0).
                                    Rectangle {
                                        z: -0.5
                                        objectName: "selectedLineBackground"
                                        property rect lineRect: {
                                            // Depend on text as well as the position: context text can change.
                                            const currentText = logText.text;
                                            return logText.positionToRectangle(Math.min(logText.length, Math.max(0, backend.selectedPosition)));
                                        }
                                        visible: backend.selectedPosition >= 0
                                        x: logText.leftPadding
                                        y: lineRect.y
                                        width: Math.max(0, logText.width - logText.leftPadding - logText.rightPadding)
                                        height: lineRect.height
                                        color: logText.palette.highlight
                                        opacity: 0.75
                                    }
                                    function followEnd() {
                                        if (follow.checked) {
                                            scroll.contentItem.contentY = Math.max(0, scroll.contentItem.contentHeight - scroll.contentItem.height);
                                        }
                                    }
                                    onTextChanged: Qt.callLater(followEnd)
                                }
                            }
                        }
                        Label {
                            text: backend.status
                            Layout.fillWidth: true
                            wrapMode: Text.WrapAnywhere
                        }
                        Label {
                            text: backend.credentialStatus
                            visible: text !== ""
                            Layout.fillWidth: true
                            wrapMode: Text.WrapAnywhere
                        }
                        ListView {
                            id: searchResults
                            ScrollBar.vertical: ScrollBar {
                                policy: ScrollBar.AlwaysOn
                            }
                            Layout.fillWidth: true
                            Layout.preferredHeight: visible ? 160 : 0
                            visible: backend.results.length > 0
                            clip: true
                            model: ListModel { id: searchResultsModel }
                            boundsBehavior: Flickable.StopAtBounds
                            flickableDirection: Flickable.VerticalFlick
                            contentWidth: width
                            function clampResultPosition() {
                                contentX = 0;
                                contentY = Math.max(originY, Math.min(contentY,
                                    originY + Math.max(0, contentHeight - height)));
                            }
                            function updateResults() {
                                const next = backend.results;
                                let identical = next.length === searchResultsModel.count;
                                for (let i = 0; identical && i < next.length; ++i) {
                                    const current = searchResultsModel.get(i).modelData;
                                    identical = current.line === next[i].line && current.text === next[i].text;
                                }
                                if (identical) return;
                                const index = indexAt(1, contentY + 1);
                                const anchor = index >= 0 ? searchResultsModel.get(index).modelData.line : -1;
                                const item = index >= 0 ? itemAtIndex(index) : null;
                                const offset = item ? item.y - contentY : 0;
                                const previousY = contentY;
                                // Search results are ordered by stable source-line number.
                                // Keep existing delegates for append-only reception updates.
                                let removed = 0;
                                while (removed < searchResultsModel.count
                                        && (next.length === 0 || searchResultsModel.get(removed).modelData.line < next[0].line))
                                    ++removed;
                                if (removed > 0) searchResultsModel.remove(0, removed);
                                let mismatch = false;
                                for (let i = 0; i < next.length; ++i) {
                                    if (i < searchResultsModel.count && searchResultsModel.get(i).modelData.line !== next[i].line) {
                                        searchResultsModel.remove(i, searchResultsModel.count - i);
                                        mismatch = true;
                                    }
                                    if (i >= searchResultsModel.count)
                                        searchResultsModel.append({modelData: {line: next[i].line, text: next[i].text}});
                                    else if (searchResultsModel.get(i).modelData.text !== next[i].text)
                                        searchResultsModel.set(i, {modelData: {line: next[i].line, text: next[i].text}});
                                }
                                if (searchResultsModel.count > next.length)
                                    searchResultsModel.remove(next.length, searchResultsModel.count - next.length);
                                forceLayout();
                                let target = -1;
                                for (let i = 0; i < next.length; ++i)
                                    if (next[i].line === anchor) { target = i; break; }
                                if (target >= 0) {
                                    positionViewAtIndex(target, ListView.Beginning);
                                    const row = itemAtIndex(target);
                                    if (row) contentY = row.y - offset;
                                } else if (removed > 0 || mismatch) {
                                    positionViewAtBeginning();
                                } else {
                                    contentY = previousY;
                                }
                                clampResultPosition();
                                // Delegate sizing and ListView's origin may settle after
                                // model removals. Clamp again without restoring stale Y.
                                Qt.callLater(searchResults.clampResultPosition);
                            }
                            Component.onCompleted: updateResults()
                            Connections {
                                target: backend
                                function onChanged() { searchResults.updateResults(); }
                            }
                            delegate: RowLayout {
                                id: resultRow
                                required property var modelData
                                width: Math.max(0, ListView.view.width - searchResults.ScrollBar.vertical.width)
                                Button {
                                    text: resultRow.modelData.line + ": ↗"
                                    Accessible.name: root.tr("Show search context")
                                    onClicked: {
                                        follow.checked = false;
                                        backend.show_context(resultRow.modelData.line);
                                    }
                                }
                                TextArea {
                                    id: resultText
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    Layout.preferredWidth: 0
                                    readOnly: true
                                    selectByMouse: true
                                    textFormat: TextEdit.RichText
                                    wrapMode: TextEdit.NoWrap
                                    font.family: "monospace"
                                    text: { const rules = page.trapRulesJson; const display = backend.displayText; return backend.highlight_result(resultRow.modelData.text); }
                                    clip: true
                                    TapHandler {
                                        acceptedButtons: Qt.RightButton
                                        onTapped: {
                                            selectionMenu.selectedText = resultText.selectedText;
                                            selectionMenu.textItem = resultText;
                                            selectionMenu.popup();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Opt-in headless integration check: two tabs, Rust/QML data and polling.
    Timer {
        id: analysisTestTimer
        interval: 100
        repeat: true
        property int ticks: 0
        property int stage: 0
        onTriggered: {
            if (++ticks > 250) { console.error("TAILER_ANALYSIS_TIMEOUT"); Qt.exit(1); return; }
            // qmllint disable missing-property
            const page = pageRepeater.itemAt(0);
            const other = pageRepeater.itemAt(1);
            if (!page || !page.backend.text.includes("/ui/policies/")) return;
            const b = page.backend;
            if (stage === 0) {
                if (analysisPresetRestoreTest) {
                    if (!page.testRestoreAnalysisPreset() || !other.testRestoreOtherAnalysisPreset()) { console.error("TAILER_ANALYSIS_PRESET_RESTORE_FAILED"); Qt.exit(1); return; }
                    console.log("TAILER_ANALYSIS_PRESET_RESTORE_OK"); Qt.quit(); return;
                }
                page.testAnalysisControls(); stage = 1; return;
            }
            if (b.analysisBusy) return;
            const r = b.analysisResult;
            if (stage === 1) {
                if (r.error || r.matched !== 2 || r.unmatched !== 4 || r.rows[0].group !== "/ui/policies/"
                    || r.rows[0].count !== 2 || other.backend.analysisResult.total !== undefined) {
                    console.error("TAILER_ANALYSIS_INITIAL_FAILED", JSON.stringify(r)); Qt.exit(1); return;
                }
                const p = b.analysis_preset(false);
                b.analyze(p.script, p.regex, "status", "", "count"); stage = 2; return;
            }
            if (stage === 2) {
                if (r.rows.length !== 2 || r.matched !== 2) { console.error("TAILER_ANALYSIS_REGROUP_FAILED"); Qt.exit(1); return; }
                b.analyze("invalid roto", "(.*)", "", "", "count"); stage = 3; return;
            }
            if (stage === 3) {
                if (!r.error || r.error.includes("\u001b")) { console.error("TAILER_ANALYSIS_ERROR_FAILED"); Qt.exit(1); return; }
                const p = b.analysis_preset(false);
                b.analyze(p.script, p.regex, "path", "", "count"); stage = 4; return;
            }
            if (stage === 4 && r.total === 7) {
                if (r.matched !== 3 || r.rows[0].count !== 3) { console.error("TAILER_ANALYSIS_LIVE_FAILED"); Qt.exit(1); return; }
                page.testAnalysisInput(3, '"event"', [
                    {name:"total", source:"/timing/total", type:"number"}
                ], "total", 1);
                stage = 5; return;
            }
            if (stage >= 5 && stage <= 7) {
                const expected = [5.442, 2.5, 3.5][stage - 5];
                if (r.error || r.matched !== 1 || r.skipped !== 6 || r.errors !== 0
                    || r.rows[0].value !== expected || r.preview.length !== 5) {
                    console.error("TAILER_ANALYSIS_INPUT_FAILED", stage, JSON.stringify(r)); Qt.exit(1); return;
                }
                if (stage === 5) {
                    page.testAnalysisInput(1, "CSV:", [{name:"total", source:"2", type:"number"}], "total", 1);
                    stage = 6; return;
                }
                if (stage === 6) {
                    page.testAnalysisInput(2, "WS:", [{name:"total", source:"2", type:"number"}], "total", 1);
                    stage = 7; return;
                }
                page.testAnalysisQuery("method, path, status, count()");
                stage = 8; return;
            }
            if (stage === 8) {
                if (r.error || r.matched !== 3 || r.rows.length !== 3
                    || JSON.stringify(r.columns) !== JSON.stringify(["method", "path", "status", "count()"])
                    || r.rows.some(row => row.cells.length !== 4 || row.cells[3] !== 1)) {
                    console.error("TAILER_ANALYSIS_QUERY_FAILED", JSON.stringify(r)); Qt.exit(1); return;
                }
                page.testAnalysisQuery("avg()"); stage = 9; return;
            }
            if (stage === 9) {
                if (!r.error || !r.error.includes("Query")) { console.error("TAILER_ANALYSIS_QUERY_ERROR_FAILED"); Qt.exit(1); return; }
                page.testAnalysisQuery("path, count()"); stage = 10; return;
            }
            if (stage === 10) {
                if (r.error || r.rows.length !== 1 || r.rows[0].cells[1] !== 3) {
                    console.error("TAILER_ANALYSIS_QUERY_RECOVERY_FAILED", JSON.stringify(r)); Qt.exit(1); return;
                }
                if (!page.testSaveAnalysisPreset()) { console.error("TAILER_ANALYSIS_PRESET_SAVE_FAILED"); Qt.exit(1); return; }
                if (!other.testSelectAnalysisPreset("builtin-apache")) { console.error("TAILER_ANALYSIS_TAB_PRESET_FAILED"); Qt.exit(1); return; }
                root.saveWorkspace();
                b.stop_analysis();
                console.log("TAILER_ANALYSIS_OK"); Qt.quit();
            }
            // qmllint enable missing-property
        }
    }
    Component.onCompleted: {
        settings.appearance = workspace.state.appearance;
        root.applyAppearance();
        const savedLanguage = workspace.state.language;
        translations.select(savedLanguage || translations.detect(Qt.locale().name));
        if (smokePath !== "") {
            addLog(smokePath);
            addLog(smokePath);
            // qmllint disable missing-property
            pageRepeater.itemAt(0).backend.set_trap("smoke-appended", false);
            pageRepeater.itemAt(1).backend.set_trap("smoke-appended", false);
            // qmllint enable missing-property
            smokeTimer.start();
        } else {
            const state = workspace.state;
            settings.initialLines = state.initialLines;
            settings.capacityMiB = state.capacityMiB;
            root.width = state.width;
            root.height = state.height;
            for (const profile of state.connections)
                connectionProfiles.append(profile);
            for (const bookmark of state.bookmarks)
                bookmarks.append(bookmark);
            for (const group of state.bookmarkGroups)
                bookmarkGroups.append(group);
            for (const tab of state.tabs)
                logs.append(tab);
            tabs.currentIndex = state.currentIndex;
        }
        restoring = false;
        if (analysisTestPath !== "") {
            if (!analysisPresetRestoreTest) {
                root.addLog(analysisTestPath);
                root.addLog(analysisTestPath);
            }
            tabs.currentIndex = 0;
            analysisTestTimer.start();
        }
        if (restoreTest)
            restoreTimer.start();
        if (connectionUiTest)
            connectionUiTimer.start();
        if (i18nTestLanguage !== "") {
            translations.select(i18nTestLanguage);
            settingsDialog.open();
            i18nTimer.start();
        }
        if (localCommandTest) {
            localCommandDialog.open();
            localCommandTitle.text = "Local command test";
            localCommandInput.text = "printf 'local-out\\n'; printf 'local-error\\n' >&2";
            localCommandTimer.start();
        }
        if (syslogTestEndpoint !== "") {
            if (logs.count === 0) {
                syslogDialog.open();
                syslogEndpoint.text = syslogTestEndpoint;
                syslogDialog.accept();
            }
            syslogTestTimer.start();
        }
        if (bookmarkTestMode !== "") bookmarkTestTimer.start();
        if (tabUiTest) {
            for (let i = 0; i < 3; ++i)
                root.addLog("printf 'tab-" + i + "\\n'; sleep 5", "custom", "Tab " + i);
            tabUiTimer.start();
        }
        if (themeTestMode !== "") {
            settingsDialog.open();
            themeTestTimer.start();
        }
    }
    Timer {
        id: tabUiTimer
        interval: 500
        property int stage: 0
        property var originalPages: []
        property var originalArchives: []
        onTriggered: {
            // qmllint disable missing-property
            if (stage === 0) {
                for (let i = 0; i < 3; ++i) {
                    originalPages.push(pageRepeater.itemAt(i));
                    originalArchives.push(pageRepeater.itemAt(i).backend.archivePath);
                }
                tabs.currentIndex = 0;
                stage = 10;
                restart();
                return;
            }
            if (stage >= 10 && stage <= 12) {
                const index = stage - 10;
                const page = pageRepeater.itemAt(index);
                if (page.backend.text !== "tab-" + index + "\n") {
                    console.error("TAILER_TAB_ACTIVATION_FAILED", index); Qt.exit(1); return;
                }
                if (stage < 12) {
                    tabs.currentIndex = index + 1;
                    stage++;
                    restart();
                    return;
                }
                tabs.currentIndex = 1;
                logs.setProperty(0, "unread", true);
                root.moveTab(0, 2);
                stage = 1;
                restart();
                return;
            }
            if (stage === 1) {
                const order = [1, 2, 0];
                for (let i = 0; i < 3; ++i) {
                    const page = pageRepeater.itemAt(i);
                    if (page !== originalPages[order[i]] || page.backend.archivePath !== originalArchives[order[i]]
                        || page.backend.text !== "tab-" + order[i] + "\n" || !page.backend.connected
                        || logs.get(i).logTitle !== "Tab " + order[i]) {
                        console.error("TAILER_TAB_MOVE_FAILED", i); Qt.exit(1); return;
                    }
                }
                if (tabs.currentIndex !== 0 || !logs.get(2).unread) { console.error("TAILER_TAB_SELECTION_FAILED"); Qt.exit(1); return; }
                root.moveTab(2, 0);
                root.moveTab(1, 2);
                root.saveWorkspace();
                if (tabs.currentIndex !== 2 || workspace.state.tabs[2].logTitle !== "Tab 1") { console.error("TAILER_TAB_ORDER_SAVE_FAILED"); Qt.exit(1); return; }
                closeTabDialog.requestClose(0, "others");
                closeTabDialog.reject();
                if (logs.count !== 3) { console.error("TAILER_TAB_CLOSE_CANCEL_FAILED"); Qt.exit(1); return; }
                // Trigger the real context menu actions on an inactive tab.
                const menu = tabRepeater.itemAt(0).contextMenu;
                for (let i = 0; i < menu.count; ++i) {
                    const action = menu.itemAt(i);
                    if (action && action.text === root.tr("Close other tabs")) action.triggered();
                }
                closeTabDialog.accept();
                if (logs.count !== 1 || logs.get(0).logTitle !== "Tab 0" || pageRepeater.itemAt(0) !== originalPages[0]
                    || !pageRepeater.itemAt(0).backend.connected || tabs.currentIndex !== 0) {
                    console.error("TAILER_TAB_CLOSE_OTHERS_FAILED"); Qt.exit(1); return;
                }
                root.addLog("printf 'extra\\n'", "custom", "Extra");
                closeTabDialog.requestClose(0);
                closeTabDialog.accept();
                if (logs.count !== 1 || logs.get(0).logTitle !== "Extra") { console.error("TAILER_TAB_CLOSE_FAILED"); Qt.exit(1); return; }
                const lastMenu = tabRepeater.itemAt(0).contextMenu;
                for (let i = 0; i < lastMenu.count; ++i) {
                    const action = lastMenu.itemAt(i);
                    if (action && action.text === root.tr("Close all tabs")) action.triggered();
                }
                closeTabDialog.accept();
                root.saveWorkspace();
                if (logs.count !== 0 || tabs.currentIndex !== -1 || workspace.state.tabs.length !== 0) {
                    console.error("TAILER_TAB_CLOSE_ALL_FAILED"); Qt.exit(1); return;
                }
                console.log("TAILER_TAB_UI_OK");
                Qt.quit();
            }
            // qmllint enable missing-property
        }
    }
    Timer {
        id: themeTestTimer
        interval: 250
        property int stage: 0
        function verify(mode) {
            if (settings.appearance !== mode || appearanceSelection.codes[appearanceSelection.currentIndex] !== mode) return false;
            const dark = mode === "dark" || (mode === "auto" && Qt.styleHints.colorScheme === Qt.Dark);
            return root.darkAppearance === dark
                && root.palette.base.toString() === (dark ? "#1b1b1b" : "#ffffff")
                && settingsDialog.palette.text.toString() === (dark ? "#eeeeee" : "#202020")
                && appearanceSelection.palette.button.toString() === (dark ? "#353535" : "#e5e5e5");
        }
        onTriggered: {
            if (root.themeTestMode !== "cycle") {
                if (!verify(root.themeTestMode)) {
                    console.error("TAILER_THEME_RESTORE_FAILED", settings.appearance, appearanceSelection.currentIndex,
                        root.darkAppearance, root.palette.base, settingsDialog.palette.text, appearanceSelection.palette.button);
                    Qt.exit(1); return;
                }
                console.log("TAILER_THEME_RESTORE_OK", root.themeTestMode);
                Qt.quit();
                return;
            }
            const modes = ["auto", "dark", "light", "auto"];
            if (!verify(modes[stage])) { console.error("TAILER_THEME_FAILED", modes[stage]); Qt.exit(1); return; }
            if (++stage < modes.length) {
                appearanceSelection.currentIndex = appearanceSelection.codes.indexOf(modes[stage]);
                appearanceSelection.activated(appearanceSelection.currentIndex);
                restart();
            } else {
                root.saveWorkspace();
                if (workspace.error !== "") { console.error("TAILER_THEME_SAVE_FAILED"); Qt.exit(1); return; }
                console.log("TAILER_THEME_CYCLE_OK");
                Qt.quit();
            }
        }
    }
    Timer {
        id: bookmarkTestTimer
        interval: 750
        property int stage: 0
        property int outputIndex: 0
        property int outputAttempts: 0
        onTriggered: {
            if (root.bookmarkTestMode === "groups-save") {
                root.editBookmarkGroup("", true);
                groupNameInput.text = "Production";
                groupEditor.accept();
                const parentId = bookmarkGroups.get(0).id;
                root.editBookmarkGroup(parentId, true);
                groupNameInput.text = "Web";
                groupEditor.accept();
                const childId = bookmarkGroups.get(1).id;
                root.editBookmarkGroup(parentId);
                groupNameInput.text = "本番";
                groupEditor.accept();
                for (let i = 0; i < 2; ++i) {
                    root.addLog("printf 'group-" + i + "\\n'", "custom", "Group log " + i);
                    root.registerBookmark(i);
                    bookmarkRegisteredDialog.accept();
                    root.editBookmark(i);
                    bookmarkGroupSelection.currentIndex = bookmarkGroupSelection.ids.indexOf(i === 0 ? parentId : childId);
                    bookmarkEditor.accept();
                }
                root.rebuildBookmarkTree();
                if (bookmarkTree.count !== 5 || bookmarkGroups.get(0).name !== "本番" || bookmarks.get(1).groupId !== childId) {
                    console.error("TAILER_GROUP_CREATE_FAILED"); Qt.exit(1); return;
                }
                root.toggleBookmarkGroup(parentId);
                if (bookmarkTree.count !== 2) { console.error("TAILER_GROUP_COLLAPSE_FAILED"); Qt.exit(1); return; }
                while (logs.count) root.closeTab(0);
                root.saveWorkspace();
                console.log("TAILER_GROUP_SAVE_OK");
                Qt.quit();
                return;
            }
            if (root.bookmarkTestMode === "groups-open") {
                const parentId = bookmarkGroups.count ? bookmarkGroups.get(0).id : "";
                if (stage === 0) {
                    root.rebuildBookmarkTree();
                    if (bookmarkGroups.count !== 2 || bookmarks.count !== 2 || bookmarkTree.count !== 2 || logs.count !== 0) {
                        console.error("TAILER_GROUP_RESTORE_FAILED"); Qt.exit(1); return;
                    }
                    root.toggleBookmarkGroup(parentId);
                    bookmarksDialog.open();
                    root.openBookmarkGroup(parentId);
                    if (!bookmarkGroupConfirm.visible || logs.count !== 0) { console.error("TAILER_GROUP_CONFIRM_FAILED"); Qt.exit(1); return; }
                    bookmarkGroupConfirm.reject();
                    if (!bookmarksDialog.visible || logs.count !== 0) { console.error("TAILER_GROUP_CANCEL_FAILED"); Qt.exit(1); return; }
                    root.openBookmarkGroup(parentId);
                    bookmarkGroupConfirm.accept();
                    if (logs.count !== 2 || bookmarksDialog.visible) { console.error("TAILER_GROUP_OPEN_FAILED"); Qt.exit(1); return; }
                    tabs.currentIndex = outputIndex;
                    stage++;
                    restart();
                    return;
                }
                // qmllint disable missing-property
                // Inactive tabs collect/archive output without refreshing display text.
                // Select each tab and allow its asynchronous view to catch up.
                const outputPage = pageRepeater.itemAt(outputIndex);
                if (!outputPage || outputPage.backend.text !== "group-" + outputIndex + "\n") {
                    if (++outputAttempts >= 10) {
                        console.error("TAILER_GROUP_OUTPUT_FAILED", outputIndex,
                            outputPage ? outputPage.backend.status : "Page missing",
                            outputPage ? outputPage.backend.text : "");
                        Qt.exit(1); return;
                    }
                    restart();
                    return;
                }
                if (++outputIndex < 2) {
                    outputAttempts = 0;
                    tabs.currentIndex = outputIndex;
                    restart();
                    return;
                }
                // qmllint enable missing-property
                while (logs.count) root.closeTab(0);
                root.deleteBookmarkGroup(parentId);
                if (bookmarkGroups.count !== 0 || bookmarks.count !== 2 || bookmarks.get(0).groupId !== "" || bookmarks.get(1).groupId !== "") {
                    console.error("TAILER_GROUP_DELETE_FAILED"); Qt.exit(1); return;
                }
                root.saveWorkspace();
                console.log("TAILER_GROUP_OPEN_DELETE_OK");
                Qt.quit();
                return;
            }
            if (root.bookmarkTestMode === "save") {
                if (stage === 0) {
                    root.addLog("printf 'bookmark-output\\n'", "custom", "Bookmark test", "CP932");
                    logs.setProperty(0, "filterText", "bookmark");
                    logs.setProperty(0, "searchText", "output");
                    logs.setProperty(0, "follow", false);
                    logs.setProperty(0, "trapRulesJson", '[{"text":"output","regex":false,"ignoreCase":true,"color":"#35c66b"}]');
                    root.registerBookmark(0);
                    root.registerBookmark(0);
                    if (bookmarks.count !== 1) { console.error("TAILER_BOOKMARK_DUPLICATE_FAILED"); Qt.exit(1); return; }
                    if (!bookmarkRegisteredDialog.visible || bookmarksDialog.visible || bookmarkRegisteredDialog.standardButtons !== Dialog.Ok) {
                        console.error("TAILER_BOOKMARK_CONFIRM_FAILED"); Qt.exit(1); return;
                    }
                    bookmarkRegisteredDialog.accept();
                    root.editBookmark(0);
                    bookmarkTitleInput.text = "  本番ログ / My bookmark  ";
                    bookmarkEditor.accept();
                    // Renaming the bookmark must not change its original configuration or duplicate it.
                    root.registerBookmark(0);
                    bookmarkRegisteredDialog.accept();
                    if (bookmarks.count !== 1 || bookmarks.get(0).bookmarkTitle !== "本番ログ / My bookmark" || logs.get(0).logTitle !== "Bookmark test") {
                        console.error("TAILER_BOOKMARK_EDIT_FAILED"); Qt.exit(1); return;
                    }
                    root.closeTab(0);
                    root.saveWorkspace();
                    if (logs.count !== 0 || bookmarks.count !== 1 || workspace.error !== "") { console.error("TAILER_BOOKMARK_SAVE_FAILED"); Qt.exit(1); return; }
                    console.log("TAILER_BOOKMARK_SAVE_OK");
                    Qt.quit();
                }
                return;
            }
            if (stage === 0) {
                if (bookmarks.count !== 1 || logs.count !== 0) { console.error("TAILER_BOOKMARK_RESTORE_FAILED"); Qt.exit(1); return; }
                if (bookmarks.get(0).bookmarkTitle !== "本番ログ / My bookmark") { console.error("TAILER_BOOKMARK_TITLE_RESTORE_FAILED"); Qt.exit(1); return; }
                bookmarksDialog.open();
                stage++;
                restart();
                return;
            }
            if (stage === 1) {
                const item = bookmarkList.itemAtIndex(1);
                item.selectBookmark(Qt.RightButton);
                const menu = item.contextMenu;
                if (!menu || !menu.visible) { console.error("TAILER_BOOKMARK_CONTEXT_FAILED"); Qt.exit(1); return; }
                menu.itemAt(2).triggered();
                menu.close();
                if (!bookmarkEditor.visible) { console.error("TAILER_BOOKMARK_CONTEXT_EDIT_FAILED"); Qt.exit(1); return; }
                bookmarkTitleInput.text = "";
                if (bookmarkEditor.standardButton(Dialog.Save).enabled) { console.error("TAILER_BOOKMARK_EMPTY_TITLE_FAILED"); Qt.exit(1); return; }
                bookmarkEditor.reject();
                item.activateBookmark(Qt.LeftButton);
                if (bookmarksDialog.visible || logs.count !== 1) { console.error("TAILER_BOOKMARK_DOUBLE_CLICK_FAILED"); Qt.exit(1); return; }
                stage++;
                restart();
                return;
            }
            // qmllint disable missing-property
            const backend = pageRepeater.itemAt(0).backend;
            const row = logs.get(0);
            if (backend.text !== "bookmark-output\n" || row.logTitle !== "本番ログ / My bookmark" || row.encoding !== "CP932" || row.filterText !== "bookmark" || row.searchText !== "output" || row.follow || JSON.parse(row.trapRulesJson)[0].text !== "output" || row.unread || row.alert) {
                console.error("TAILER_BOOKMARK_OPEN_FAILED", backend.text); Qt.exit(1); return;
            }
            // qmllint enable missing-property
            const remote = root.logConfiguration(row);
            remote.remote = true;
            remote.connectionId = "bookmark-profile";
            remote.bookmarkTitle = "Remote test";
            bookmarks.append(remote);
            if (!root.profileUsed("bookmark-profile")) { console.error("TAILER_BOOKMARK_PROFILE_FAILED"); Qt.exit(1); return; }
            bookmarks.remove(1);
            root.closeTab(0);
            root.deleteBookmark(0);
            root.saveWorkspace();
            if (workspace.error !== "" || root.profileUsed("bookmark-profile")) { console.error("TAILER_BOOKMARK_DELETE_FAILED"); Qt.exit(1); return; }
            console.log("TAILER_BOOKMARK_OPEN_DELETE_OK");
            Qt.quit();
        }
    }
    Timer {
        id: syslogTestTimer
        interval: 100
        repeat: true
        property int attempts: 0
        property int stage: 0
        onTriggered: {
            // qmllint disable missing-property
            const page = pageRepeater.itemAt(0);
            if (++attempts > 150 || !page || !page.backend.connected) {
                console.error("TAILER_SYSLOG_FAILED", page ? page.backend.status : "missing page"); Qt.exit(1); return;
            }
            const b = page.backend;
            if (stage === 0) {
                if (!b.text.includes("%LINK-3-UPDOWN") || !b.text.includes("RPD_BGP")
                    || !b.text.includes("bsd su:") || !b.text.includes("日本語")) return;
                const raw = b.text;
                page.testSyslogLabels(true);
                if (b.text !== raw || !b.displayText.includes("[NOTI|LOC7]") || b.displayText.includes("<189>")) {
                    console.error("TAILER_SYSLOG_LABELS_FAILED"); Qt.exit(1); return;
                }
                page.testSyslogLabels(false);
                if (b.displayText !== raw) { console.error("TAILER_SYSLOG_LABELS_OFF_FAILED"); Qt.exit(1); return; }
                page.testSyslogLabels(true);
                b.analyze_input('fn parse(c: Captures) -> Record { Record.new().text("severity", syslog_severity(c.text("raw"))) }',
                    "^(?P<raw>.*)$", "", "", "count", {format:"regex", fields:[], query:"severity, count()"});
                stage = 1;
            } else if (stage === 1) {
                if (b.analysisBusy) return;
                if (!page.logText || !page.logText.text.includes("[NOTI|LOC4]")) {
                    console.error("TAILER_SYSLOG_RENDER_FAILED"); Qt.exit(1); return;
                }
                if (b.analysisResult.error || b.analysisResult.matched !== 4) {
                    console.error("TAILER_SYSLOG_ANALYSIS_FAILED", JSON.stringify(b.analysisResult)); Qt.exit(1); return;
                }
                b.stop_analysis();
                b.set_trap_rules([{text:"[NOTI|LOC7]", color:"#abcdef"}]);
                if (!b.highlightedText.includes('color:#abcdef;font-weight:bold">[NOTI|LOC7]')) {
                    console.error("TAILER_SYSLOG_TRAP_LABEL_FAILED"); Qt.exit(1); return;
                }
                b.search("[NOTI|LOC7]", false, false);
                stage = 2;
            } else if (stage === 2) {
                if (b.results.length !== 1) return;
                b.set_filter("[NOTI|LOC7]", false, false, false);
                stage = 10;
            } else if (stage === 10) {
                if (!b.summary.includes("Filter matched 1 lines")) return;
                if (!b.text.includes("%LINK-3-UPDOWN") || !b.displayText.startsWith("[NOTI|LOC7]")) {
                    console.error("TAILER_SYSLOG_LABEL_FILTER_FAILED"); Qt.exit(1); return;
                }
                page.testSyslogLabels(false);
                stage = 11;
            } else if (stage === 11) {
                if (!b.summary.includes("Filter matched 0 lines")) return;
                if (b.text !== "") { console.error("TAILER_SYSLOG_RAW_FILTER_FAILED"); Qt.exit(1); return; }
                if (b.results.length !== 0) return;
                b.search("<189>", false, false);
                b.set_filter("<189>", false, false, false);
                stage = 12;
            } else if (stage === 12) {
                if (!b.summary.includes("Filter matched 1 lines")) return;
                if (b.results.length !== 1) return;
                if (!b.text.startsWith("<189>")) { console.error("TAILER_SYSLOG_PRI_FILTER_FAILED"); Qt.exit(1); return; }
                b.set_filter("", false, false, false);
                page.testSyslogLabels(true);
                editTabDialog.edit(0);
                if (editSource.currentIndex !== 3 || editLogPath.text !== root.syslogTestEndpoint) {
                    console.error("TAILER_SYSLOG_EDIT_FAILED"); Qt.exit(1); return;
                }
                editTabDialog.accept();
                stage = 3;
            } else {
                root.saveWorkspace();
                if (workspace.state.tabs[0].source !== "syslog-udp" || workspace.state.tabs[0].logPath !== root.syslogTestEndpoint
                    || !workspace.state.tabs[0].syslogBadges) {
                    console.error("TAILER_SYSLOG_SAVE_FAILED"); Qt.exit(1); return;
                }
                stop();
                b.stop();
                console.log("TAILER_SYSLOG_OK"); Qt.quit();
            }
            // qmllint enable missing-property
        }
    }
    Timer {
        id: localCommandTimer
        interval: 500
        property int stage: 0
        onTriggered: {
            if (stage === 0) {
                localCommandDialog.accept();
                stage++;
                restart();
            } else if (stage === 1) {
                closeTabDialog.tabIndex = 0;
                closeTabDialog.open();
                stage = 10;
                restart();
            } else if (stage === 10) {
                closeTabDialog.reject();
                if (logs.count !== 1) { console.error("TAILER_CLOSE_CANCEL_FAILED"); Qt.exit(1); return; }
                // qmllint disable missing-property
                const page = pageRepeater.itemAt(0);
                if (!page || page.backend.text !== "local-out\nlocal-error\n" || logs.get(0).remote || logs.get(0).source !== "custom" || !page.testManualScroll() || logs.get(0).follow) {
                    console.error("TAILER_LOCAL_COMMAND_FAILED"); Qt.exit(1); return;
                }
                const tab = tabRepeater.itemAt(0);
                const fakeStatuses = {test: "ssh_srv: Connected"};
                if (workspaceFooter.statusText({remote: false, connectionId: "test"}, fakeStatuses).includes("ssh_srv")
                    || workspaceFooter.statusText({remote: true, connectionId: "other"}, fakeStatuses).includes("ssh_srv")
                    || workspaceFooter.statusText({remote: true, connectionId: "test"}, fakeStatuses) !== root.tr("SSH transport") + ": ssh_srv: Connected") {
                    console.error("TAILER_FOOTER_SCOPE_FAILED"); Qt.exit(1); return;
                }
                tabs.currentIndex = -1;
                logs.setProperty(0, "unread", true);
                tabs.currentIndex = 0;
                if (page.backend.connected || tab.connected || !tab.statusDot.visible || tab.statusDot.color.toString() !== "#e05252") {
                    console.error("TAILER_DISCONNECTED_DOT_FAILED"); Qt.exit(1); return;
                }
                // qmllint enable missing-property
                root.saveWorkspace();
                if (workspace.state.tabs[0].source !== "custom" || workspace.state.tabs[0].follow) {
                    console.error("TAILER_LOCAL_COMMAND_SAVE_FAILED"); Qt.exit(1); return;
                }
                editTabDialog.edit(0);
                editLogPath.text = "printf 'edited-out\\n'";
                editLogTitle.text = "Edited command";
                editTabDialog.accept();
                stage = 2;
                restart();
            } else {
                // qmllint disable missing-property
                const page = pageRepeater.itemAt(0);
                if (logs.count !== 1 || logs.get(0).logTitle !== "Edited command" || page.backend.text !== "edited-out\n" || page.backend.connected) {
                    console.error("TAILER_EDIT_RECONNECT_FAILED"); Qt.exit(1); return;
                }
                const archive = page.backend.archivePath;
                editTabDialog.edit(0);
                editEncoding.currentIndex = root.encodingOptions.indexOf("CP932");
                editTabDialog.accept();
                if (page.backend.archivePath !== archive || logs.get(0).encoding !== "CP932" || page.backend.text !== "edited-out\n") {
                    console.error("TAILER_ENCODING_RESTARTED_COMMAND"); Qt.exit(1); return;
                }
                // qmllint enable missing-property
                root.saveWorkspace();
                console.log("TAILER_LOCAL_COMMAND_OK");
                Qt.quit();
            }
        }
    }
    Timer {
        id: i18nTimer
        interval: 500
        property int stage: 0
        onTriggered: {
            if (stage === 0) {
                if (settingsDialog.title !== root.tr("Settings") || settingsDialog.standardButton(Dialog.Save).text !== root.tr("Save") || languageSelection.codes[languageSelection.currentIndex] !== root.i18nTestLanguage) {
                    console.error("TAILER_I18N_FAILED", root.i18nTestLanguage, settingsDialog.title); Qt.exit(1); return;
                }
                translations.select("ja");
                stage++;
                restart();
            } else if (stage === 1) {
                if (settingsDialog.title !== "取得設定" || settingsDialog.standardButton(Dialog.Save).text !== "保存") {
                    console.error("TAILER_I18N_SWITCH_FAILED"); Qt.exit(1); return;
                }
                translations.select(root.i18nTestLanguage);
                stage++;
                restart();
            } else {
                root.saveWorkspace();
                if (workspace.state.language !== root.i18nTestLanguage) {
                    console.error("TAILER_I18N_SAVE_FAILED"); Qt.exit(1); return;
                }
                console.log("TAILER_I18N_OK", root.i18nTestLanguage);
                Qt.quit();
            }
        }
    }
    Timer {
        id: connectionUiTimer
        interval: 500
        property int stage: 0
        repeat: true
        onTriggered: {
            if (stage === 0) {
                profilesDialog.open();
                profileEditor.edit(-1);
                profileName.text = "UI test";
                profileHost.text = "localhost";
            } else if (stage === 1) {
                if (!profileEditor.standardButton(Dialog.Save).enabled) {
                    console.error("TAILER_CONNECTION_UI_FAILED save", profileName.text, profileHost.text); Qt.exit(1); return;
                }
                profileEditor.accept();
                profilesDialog.close();
                sshDialog.open();
            } else if (stage === 2) {
                if (connectionProfiles.count !== 1 || sshProfile.currentIndex !== 0 || !sshDialog.standardButton(Dialog.Ok).enabled || root.profileById(connectionProfiles.get(0).id).host !== "localhost") {
                    console.error("TAILER_CONNECTION_UI_FAILED selection", connectionProfiles.count, sshProfile.currentIndex, sshDialog.standardButton(Dialog.Ok).enabled, JSON.stringify(connectionProfiles.get(0))); Qt.exit(1); return;
                }
                sshDialog.reject();
                sshSource.currentIndex = 2;
                sshDialog.open();
            } else if (stage === 3) {
                if (sshDialog.standardButton(Dialog.Ok).enabled || !sshCommand.visible || sshLines.visible) {
                    console.error("TAILER_CUSTOM_UI_FAILED empty"); Qt.exit(1); return;
                }
                sshCommand.text = "printf 'custom\\n' | cat\nprintf 'error\\n' >&2";
                sshCommandTitle.text = "Custom test";
                sshEncoding.currentIndex = 1;
            } else if (stage === 4) {
                if (!sshDialog.standardButton(Dialog.Ok).enabled || sshEncoding.currentText !== "CP932") {
                    console.error("TAILER_CUSTOM_UI_FAILED command"); Qt.exit(1); return;
                }
                sshDialog.reject();
                console.log("TAILER_CUSTOM_UI_OK");
                console.log("TAILER_CONNECTION_UI_OK");
                Qt.quit();
            }
            stage++;
        }
    }
    Timer {
        id: restoreTimer
        interval: 3000
        property bool activated: false
        onTriggered: {
            if (!activated) {
                if (tabs.currentIndex !== 1) { console.error("TAILER_RESTORE_SELECTION_FAILED"); Qt.exit(1); return; }
                tabs.currentIndex = 0;
                activated = true;
                restart();
                return;
            }
            // Integration fixture has two local tabs, including a saved filter.
            // qmllint disable missing-property
            const page = pageRepeater.itemAt(0);
            const expected = logs.count > 0 && logs.get(0).encoding === "CP932" ? "ERROR 日本語" : "ERROR old";
            if (logs.count !== 2 || tabs.currentIndex !== 0 || !page || page.backend.text.trim() !== expected || logs.get(0).filterText !== "ERROR" || settings.initialLines !== 20) {
                console.error("TAILER_RESTORE_FAILED");
                Qt.exit(1);
                return;
            }
            if (page.trapRules.length > 0 && !page.backend.highlightedText.includes("color:" + page.trapRules[0].color)) {
                console.error("TAILER_RESTORE_TRAP_FAILED"); Qt.exit(1); return;
            }
            // qmllint enable missing-property
            tabs.currentIndex = 1;
            root.saveWorkspace();
            console.log("TAILER_RESTORE_OK");
            Qt.quit();
        }
    }
    Timer {
        id: smokeTimer
        interval: 1000
        repeat: true
        onTriggered: {
            root.smokeStage++;
            if (root.smokeStage < 4)
                return;
            for (let i = 0; i < 2; ++i) {
                const page = pageRepeater.itemAt(i);
                // The Repeater returns QQuickItem; these properties belong to its delegate.
                // qmllint disable missing-property
                if (!page || (root.smokeStage === 4 && i === 1 && !page.backend.text.includes("smoke-appended"))) {
                    console.error("TAILER_SMOKE_FAILED", i);
                    Qt.exit(1);
                    return;
                }
                // qmllint enable missing-property
            }
            // qmllint disable missing-property
            if (root.smokeStage === 4) {
                const inactiveTab = tabRepeater.itemAt(0);
                const activeTab = tabRepeater.itemAt(1);
                if (!inactiveTab.connected || !inactiveTab.statusDot.visible || inactiveTab.statusDot.color.toString() !== "#35c66b" || activeTab.statusDot.visible) {
                    console.error("TAILER_CONNECTED_DOT_FAILED"); Qt.exit(1); return;
                }
                if (!logs.get(0).unread || logs.get(1).unread || !logs.get(0).alert || logs.get(1).alert) {
                    console.error("TAILER_UNREAD_FAILED", logs.get(0).unread, logs.get(1).unread, logs.get(0).alert, logs.get(1).alert, pageRepeater.itemAt(0).backend.highlightedText);
                    Qt.exit(1);
                    return;
                }
                if (!pageRepeater.itemAt(1).testOptionsPanel()) {
                    console.error("TAILER_OPTIONS_PANEL_FAILED");
                    Qt.exit(1);
                    return;
                }
                tabs.currentIndex = 0;
                if (logs.get(0).unread || logs.get(0).alert) {
                    console.error("TAILER_UNREAD_CLEAR_FAILED");
                    Qt.exit(1);
                    return;
                }
                pageRepeater.itemAt(0).backend.set_filter("smoke-initial", false, false, false);
                pageRepeater.itemAt(0).backend.search("smoke-appended", false, false);
                return;
            }
            const first = pageRepeater.itemAt(0).backend;
            if (root.smokeStage === 6) {
                if (!first.highlightedText.includes("color:#35c66b")) {
                    console.error("TAILER_HIGHLIGHT_FAILED"); Qt.exit(1); return;
                }
                if (first.selectedPosition < 0 || !first.text.includes("2: smoke-appended")) {
                    console.error("TAILER_CONTEXT_HIGHLIGHT_FAILED"); Qt.exit(1); return;
                }
                first.search("smoke-appended", false, false);
                return;
            }
            if (root.smokeStage === 7) {
                if (first.selectedPosition !== -1 || !first.text.includes("smoke-initial") || first.text.includes("2: smoke-appended") || first.results.length !== 1) {
                    console.error("TAILER_SEARCH_RESET_CONTEXT_FAILED"); Qt.exit(1); return;
                }
                if (!pageRepeater.itemAt(0).testTrapTags() || logs.get(1).trapRulesJson !== "[]") {
                    console.error("TAILER_TRAP_TAGS_FAILED"); Qt.exit(1); return;
                }
                root.closeTab(0);
                console.log("TAILER_SMOKE_OK");
                Qt.quit();
                return;
            }
            if (logs.get(0).unread) {
                console.error("TAILER_FILTER_MARKED_UNREAD");
                Qt.exit(1);
                return;
            }
            if (!first.text.includes("smoke-initial") || first.text.includes("smoke-appended") || first.results.length !== 1) {
                console.error("TAILER_SEARCH_SMOKE_FAILED");
                Qt.exit(1);
                return;
            }
            // qmllint enable missing-property
            first.show_context(2);
        }
    }
}
