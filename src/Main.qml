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
    property bool restoreTest: false
    property bool connectionUiTest: false
    property string i18nTestLanguage: ""
    property bool localCommandTest: false
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
        return false;
    }
    property bool restoring: true
    property var encodingOptions: ["UTF-8", "CP932", "EUC-JP", "ISO-2022-JP", "UTF-16LE", "UTF-16BE", "GB18030", "BIG5", "CP949", "WINDOWS-1252", "ISO-8859-1"]
    QtObject {
        id: settings
        property int initialLines: 50
        property int capacityMiB: 512
    }
    function scheduleSave() {
        if (!restoring && smokePath === "")
            saveTimer.restart();
    }
    function saveWorkspace() {
        if (restoring || smokePath !== "")
            return;
        const saved = [];
        for (let i = 0; i < logs.count; ++i) {
            const row = logs.get(i);
            saved.push({
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
                filterRegex: row.filterRegex,
                filterCase: row.filterCase,
                filterInvert: row.filterInvert,
                searchText: row.searchText,
                searchRegex: row.searchRegex,
                searchCase: row.searchCase,
                follow: row.follow,
                trapRulesJson: row.trapRulesJson
            });
        }
        const profiles = [];
        for (let i = 0; i < connectionProfiles.count; ++i) {
            const p = connectionProfiles.get(i);
            profiles.push({id: p.id, name: p.name, host: p.host, user: p.user, port: p.port, key: p.key});
        }
        workspace.save({
            version: 2,
            language: translations.language,
            connections: profiles,
            tabs: saved,
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
            filterRegex: false,
            filterCase: false,
            filterInvert: false,
            searchText: "",
            searchRegex: false,
            searchCase: false,
            follow: true
        });
        tabs.currentIndex = logs.count - 1;
        scheduleSave();
    }

    function closeTab(index) {
        // qmllint disable missing-property
        const page = pageRepeater.itemAt(index);
        if (page)
            page.backend.stop();
        // qmllint enable missing-property
        const next = Math.max(0, Math.min(tabs.currentIndex, logs.count - 2));
        logs.remove(index);
        tabs.currentIndex = next;
        scheduleSave();
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

    Dialog {
        id: closeTabDialog
        property int tabIndex: -1
        function requestClose(index) {
            if (index < 0 || index >= logs.count) return;
            tabIndex = index;
            open();
        }
        title: root.tr("Close log tab?")
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.CloseOnEscape
        standardButtons: Dialog.Ok | Dialog.Cancel
        onOpened: root.translateButtons(closeTabDialog)
        onAccepted: { if (tabIndex >= 0 && tabIndex < logs.count) root.closeTab(tabIndex); }
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
            editSource.currentIndex = row.source === "custom" ? 2 : (row.source === "docker" ? 1 : 0);
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
            const encodingOnly = previous.source === ["file", "docker", "custom"][editSource.currentIndex]
                && previous.logPath === editLogPath.text
                && (!remote || previous.connectionId === connectionProfiles.get(editProfile.currentIndex).id)
                && previous.elevation === editElevation.currentIndex && previous.runUser === editRunUser.text
                && previous.initial === editInitial.value && previous.capacity === editCapacity.value
                && previous.encoding !== editEncoding.currentText;
            logs.setProperty(index, "logTitle", editLogTitle.text.trim() || logs.get(index).logTitle);
            logs.setProperty(index, "source", ["file", "docker", "custom"][editSource.currentIndex]);
            logs.setProperty(index, "logPath", editLogPath.text);
            logs.setProperty(index, "encoding", editEncoding.currentText);
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
                model: [root.tr("File (tail)"), root.tr("Docker container"), root.tr("Custom command")]
                Layout.fillWidth: true
                onActivated: { if (!editTabDialog.remote && currentIndex === 1) currentIndex = 0; }
            }
            Label { visible: editTabDialog.remote; text: root.tr("Saved connection") }
            ComboBox { id: editProfile; visible: editTabDialog.remote; model: connectionProfiles; textRole: "name"; Layout.fillWidth: true }
            Label { text: root.tr("Log path, container or command") }
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: editSource.currentIndex === 2 ? 120 : 60
                TextArea { id: editLogPath; wrapMode: TextEdit.Wrap; selectByMouse: true }
            }
            RowLayout {
                Label { text: root.tr("Input encoding") }
                ComboBox { id: editEncoding; model: root.encodingOptions; Layout.fillWidth: true }
            }
            RowLayout {
                visible: editTabDialog.remote
                Label { text: root.tr("Privilege escalation") }
                ComboBox { id: editElevation; model: [root.tr("None"), "sudo", root.tr("su (login)")] }
                TextField { id: editRunUser; enabled: editElevation.currentIndex !== 0; placeholderText: root.tr("Run as user"); Layout.fillWidth: true }
            }
            RowLayout {
                visible: editSource.currentIndex !== 2
                Label { text: root.tr("Initial tail lines") }
                SpinBox { id: editInitial; from: 0; to: 1000000; editable: true }
            }
            Label { text: root.tr("Storage limit in MiB per tab (stops when reached)") }
            SpinBox { id: editCapacity; from: 1; to: 10240; editable: true }
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
                text: root.tr("Changes apply on the next connection. Connections used by tabs cannot be deleted.")
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
        }
        onAccepted: {
            settings.initialLines = defaultLines.value;
            settings.capacityMiB = defaultCapacity.value;
            root.scheduleSave();
        }
        ColumnLayout {
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
                text: root.tr("Storage limit in MiB per tab (stops when reached)")
            }
            SpinBox {
                id: defaultCapacity
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
                filterRegex: false,
                filterCase: false,
                filterInvert: false,
                searchText: "",
                searchRegex: false,
                searchCase: false,
                follow: true
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
                text: root.tr("Connections…")
                onClicked: profilesDialog.open()
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
            onCurrentIndexChanged: {
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
                    text: logTitle
                    rightPadding: 36
                    implicitHeight: Math.max(implicitBackgroundHeight + topInset + bottomInset,
                                             implicitContentHeight + topPadding + bottomPadding) + 3
                    background: Rectangle {
                        implicitHeight: 21
                        color: tab.checked ? Qt.lighter(tab.palette.button, 1.50) : (tab.hovered ? Qt.lighter(tab.palette.button, 1.08) : tab.palette.button)
                        border.color: tab.palette.mid
                        radius: 2
                    }
                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        onTapped: tabMenu.popup()
                    }
                    Menu {
                        id: tabMenu
                        MenuItem { text: root.tr("Edit and reconnect…"); onTriggered: editTabDialog.edit(tab.index) }
                        MenuItem { text: root.tr("Reconnect / run again"); onTriggered: root.restartTab(tab.index) }
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
                    required property string filterText
                    required property bool filterRegex
                    required property bool filterCase
                    required property bool filterInvert
                    required property string searchText
                    required property bool searchRegex
                    required property bool searchCase
                    required property bool follow
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
                    function editTrap(tagIndex, selectedText = "") {
                        trapEditor.tagIndex = tagIndex;
                        const rule = tagIndex >= 0 ? page.trapRules[tagIndex]
                            : {text: selectedText, regex: false, ignoreCase: false, color: "#35c66b"};
                        editTrapText.text = rule.text;
                        editTrapRegex.checked = rule.regex;
                        editTrapCase.checked = rule.ignoreCase;
                        editTrapColor.text = rule.color;
                        trapEditor.error = "";
                        trapEditor.open();
                    }
                    property alias backend: backend
                    function restartSource() {
                        backend.stop();
                        backend.set_encoding(page.encoding);
                        if (page.remote)
                            backend.connect_profile(root.profileById(page.connectionId), page.logPath, page.initial, page.capacity, page.elevation, page.runUser, page.source);
                        else if (page.source === "custom")
                            backend.run_local(page.logPath, page.capacity);
                        else
                            backend.open(page.logPath, page.initial, page.capacity);
                        backend.set_filter(filterInput.text, filterRegex.checked, filterCase.checked, filterInvert.checked);
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
                        optionsToggle.toggle();
                        const retained = !filterRow.visible && !searchRow.visible && !querySummary.visible && !trapRow.visible
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
                        logs.setProperty(index, "filterRegex", filterRegex.checked);
                        logs.setProperty(index, "filterCase", filterCase.checked);
                        logs.setProperty(index, "filterInvert", filterInvert.checked);
                        logs.setProperty(index, "searchText", searchInput.text);
                        logs.setProperty(index, "searchRegex", searchRegex.checked);
                        logs.setProperty(index, "searchCase", searchCase.checked);
                        logs.setProperty(index, "follow", follow.checked);
                        root.scheduleSave();
                    }
                    Component.onCompleted: viewReady = true

                    LogBackend {
                        id: backend
                        Component.onCompleted: {
                            set_encoding(page.encoding);
                            if (page.remote)
                                connect_profile(root.profileById(page.connectionId), page.logPath, page.initial, page.capacity, page.elevation, page.runUser, page.source);
                            else if (page.source === "custom")
                                run_local(page.logPath, page.capacity);
                            else
                                open(page.logPath, page.initial, page.capacity);
                            set_filter(page.filterText, page.filterRegex, page.filterCase, page.filterInvert);
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
                            const rule = {text: editTrapText.text, regex: editTrapRegex.checked,
                                ignoreCase: editTrapCase.checked, color: editTrapColor.text};
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
                                CheckBox { id: editTrapRegex; text: root.tr("Regex") }
                                CheckBox { id: editTrapCase; text: root.tr("Ignore case") }
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
                                        logText.deselect();
                                        Qt.callLater(scroll.updateLogText);
                                        Qt.callLater(logText.followEnd);
                                    }
                                }
                            }
                            ToolButton {
                                id: optionsToggle
                                checkable: true
                                checked: false
                                text: {
                                    const active = [];
                                    if (filterInput.text !== "") active.push(root.tr("Filter enabled"));
                                    if (backend.results.length > 0) active.push(root.tr("Search results"));
                                    if (page.trapRules.length > 0) active.push(root.tr("Text trap enabled"));
                                    return (checked ? "▼ " : "▶ ") + root.tr("Options")
                                        + (active.length > 0 ? " (" + active.join(", ") + ")" : "");
                                }
                                Accessible.name: text
                            }
                        }
                        RowLayout {
                            id: filterRow
                            visible: optionsToggle.checked
                            TextField {
                                id: filterInput
                                text: page.filterText
                                placeholderText: root.tr("Filter (all history)")
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
                            Button {
                                text: root.tr("Go live")
                                onClicked: backend.show_live()
                            }
                        }
                        Timer {
                            id: filterTimer
                            interval: 300
                            onTriggered: backend.set_filter(filterInput.text, filterRegex.checked, filterCase.checked, filterInvert.checked)
                        }
                        RowLayout {
                            id: searchRow
                            visible: optionsToggle.checked
                            TextField {
                                id: searchInput
                                text: page.searchText
                                onTextEdited: page.saveView()
                                placeholderText: root.tr("Search all history (independent of the filter)")
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
                            property real savedY: 0
                            property real savedX: 0
                            property bool updatingText: false
                            property string appliedText: ""
                            function updateLogText() {
                                // setText resets Qt's selection and mouse-drag anchor. Keep
                                // this document stable until selection/copying is finished.
                                // Reception, decoding, archiving and traps continue in Rust.
                                if (selectionPointer.active || logText.selectionStart !== logText.selectionEnd)
                                    return;
                                if (appliedText === backend.highlightedText) return;
                                const y = savedY;
                                const x = savedX;
                                updatingText = true;
                                appliedText = backend.highlightedText;
                                logText.text = appliedText;
                                Qt.callLater(() => {
                                    if (follow.checked) logText.followEnd();
                                    else {
                                        scroll.contentItem.contentY = Math.max(0, Math.min(y, scroll.contentItem.contentHeight - scroll.contentItem.height));
                                        scroll.contentItem.contentX = Math.max(0, Math.min(x, scroll.contentItem.contentWidth - scroll.contentItem.width));
                                    }
                                    scroll.updatingText = false;
                                    scroll.savedY = scroll.contentItem.contentY;
                                    scroll.savedX = scroll.contentItem.contentX;
                                });
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
                            TextArea {
                                id: logText
                                text: ""
                                readOnly: true
                                Keys.onPressed: event => {
                                    if (event.key === Qt.Key_Up || event.key === Qt.Key_PageUp || event.key === Qt.Key_Home)
                                        scroll.pauseFollow();
                                    event.accepted = false;
                                }
                                textFormat: TextEdit.RichText
                                wrapMode: TextEdit.NoWrap
                                font.family: "monospace"
                                selectByMouse: true
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
                                    Rectangle {
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
                                }
                                function followEnd() {
                                    if (follow.checked) {
                                        scroll.contentItem.contentY = Math.max(0, scroll.contentItem.contentHeight - scroll.contentItem.height);
                                    }
                                }
                                onTextChanged: Qt.callLater(followEnd)
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
                            Layout.fillWidth: true
                            Layout.preferredHeight: visible ? 160 : 0
                            visible: backend.results.length > 0
                            clip: true
                            model: backend.results
                            boundsBehavior: Flickable.StopAtBounds
                            delegate: RowLayout {
                                id: resultRow
                                required property var modelData
                                width: ListView.view.width
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
                                    readOnly: true
                                    selectByMouse: true
                                    textFormat: TextEdit.RichText
                                    wrapMode: TextEdit.NoWrap
                                    font.family: "monospace"
                                    text: { const rules = page.trapRulesJson; return backend.highlight_result(resultRow.modelData.text); }
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
                            // WheelHandler {
                            //     acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                            //     onWheel: (event) => {
                            //         let scrollFactor = 2.0;
                            //         listView.contentY -= (event.angleDelta.y * scrollFactor);
                            //         event.accepted = true;
                            //     }
                            // }
                        }
                    }
                }
            }
        }
    }

    // Opt-in headless integration check: two tabs, Rust/QML data and polling.
    Component.onCompleted: {
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
            for (const tab of state.tabs)
                logs.append(tab);
            tabs.currentIndex = state.currentIndex;
        }
        restoring = false;
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
        onTriggered: {
            // Integration fixture has two local tabs, including a saved filter.
            // qmllint disable missing-property
            const page = pageRepeater.itemAt(0);
            const expected = logs.count > 0 && logs.get(0).encoding === "CP932" ? "ERROR 日本語" : "ERROR old";
            if (logs.count !== 2 || tabs.currentIndex !== 1 || !page || page.backend.text.trim() !== expected || logs.get(0).filterText !== "ERROR" || settings.initialLines !== 20) {
                console.error("TAILER_RESTORE_FAILED");
                Qt.exit(1);
                return;
            }
            if (page.trapRules.length > 0 && !page.backend.highlightedText.includes("color:" + page.trapRules[0].color)) {
                console.error("TAILER_RESTORE_TRAP_FAILED"); Qt.exit(1); return;
            }
            // qmllint enable missing-property
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
                if (!page || (root.smokeStage === 4 && !page.backend.text.includes("smoke-appended"))) {
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
                tabs.currentIndex = 0;
                if (logs.get(0).unread || logs.get(0).alert) {
                    console.error("TAILER_UNREAD_CLEAR_FAILED");
                    Qt.exit(1);
                    return;
                }
                tabs.currentIndex = 1;
                if (!pageRepeater.itemAt(1).testOptionsPanel()) {
                    console.error("TAILER_OPTIONS_PANEL_FAILED");
                    Qt.exit(1);
                    return;
                }
                if (!pageRepeater.itemAt(0).backend.highlightedText.includes("color:#35c66b")) {
                    console.error("TAILER_HIGHLIGHT_FAILED");
                    Qt.exit(1);
                    return;
                }
                pageRepeater.itemAt(0).backend.set_filter("smoke-initial", false, false, false);
                pageRepeater.itemAt(0).backend.search("smoke-appended", false, false);
                return;
            }
            const first = pageRepeater.itemAt(0).backend;
            if (root.smokeStage === 6) {
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
