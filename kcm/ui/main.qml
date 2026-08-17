import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kcmutils as KCM
import org.kde.kirigami as Kirigami
import org.kde.prison as Prison

KCM.ScrollViewKCM {
    id: root

    readonly property var devices: kcm.devices

    title: i18n("Phone Passkey")

    // kcmshell6 sizes its window from the module's implicit size; without this it opens
    // ~360px tall and every dialog is clamped to that.
    implicitWidth: Kirigami.Units.gridUnit * 42
    implicitHeight: Kirigami.Units.gridUnit * 32

    Timer {
        // Drives the pairing countdown; the daemon owns the deadline, this only displays it.
        interval: 1000
        running: root.devices.pairing
        repeat: true
        onTriggered: pairingSheet.secondsLeft = root.devices.pairingSecondsLeft()
    }

    header: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: !root.devices.available
            type: Kirigami.MessageType.Error
            text: i18n("The kpasskey service is not running, so phones cannot be paired or removed.")
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: root.devices.available && root.devices.statusMessage.length > 0
            type: Kirigami.MessageType.Information
            text: root.devices.statusMessage
        }
    }

    view: ListView {
        id: deviceList

        model: root.devices
        currentIndex: -1

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - (Kirigami.Units.largeSpacing * 4)
            visible: deviceList.count === 0
            icon.name: "smartphone"
            text: i18n("No phone paired")
            explanation: i18n("Pair a phone to use it as a passkey for unlocking this computer and authorising administrator actions.")

            helpfulAction: Kirigami.Action {
                icon.name: "list-add"
                text: i18n("Pair a phone…")
                enabled: root.devices.available
                onTriggered: root.startPairing()
            }
        }

        // A plain delegate with an explicit button: SwipeListItem overlays its actions on
        // top of the content, which collides with the security badge.
        delegate: QQC2.ItemDelegate {
            id: item

            required property string deviceId
            required property string name
            required property string model
            required property string securityLevel
            required property string pairedAt
            required property bool hardwareBacked
            required property string fingerprint

            width: deviceList.width
            hoverEnabled: false
            down: false

            contentItem: RowLayout {
                spacing: Kirigami.Units.largeSpacing

                Kirigami.Icon {
                    source: "smartphone"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: item.name
                        elide: Text.ElideRight
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: i18n("%1 · paired %2", item.model, item.pairedAt)
                        elide: Text.ElideRight
                        font: Kirigami.Theme.smallFont
                        opacity: 0.7
                    }

                    // Read off the phone's own screen and compared by eye, so it is spaced
                    // to be read a character at a time rather than skimmed.
                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        text: item.fingerprint
                        elide: Text.ElideRight
                        font.pointSize: Kirigami.Theme.smallFont.pointSize
                        font.letterSpacing: Kirigami.Units.smallSpacing / 2
                        opacity: 0.9
                    }
                }

                RowLayout {
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Icon {
                        // The key is only trustworthy if attestation said it lives in hardware.
                        source: item.hardwareBacked ? "security-high" : "security-low"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }

                    QQC2.Label {
                        text: item.hardwareBacked ? i18n("Hardware-backed (%1)", item.securityLevel)
                                                  : i18n("Not hardware-backed")
                        font: Kirigami.Theme.smallFont
                        opacity: 0.8
                    }
                }

                QQC2.Button {
                    text: i18n("Remove")
                    icon.name: "edit-delete"
                    display: QQC2.AbstractButton.IconOnly
                    onClicked: {
                        confirmRemoval.deviceId = item.deviceId;
                        confirmRemoval.deviceName = item.name;
                        confirmRemoval.open();
                    }

                    QQC2.ToolTip.visible: hovered
                    QQC2.ToolTip.text: i18n("Remove this phone")
                }
            }
        }
    }

    footer: RowLayout {
        QQC2.Button {
            text: i18n("Pair a phone…")
            icon.name: "list-add"
            // The empty-state placeholder already offers this, front and centre.
            visible: deviceList.count > 0
            enabled: root.devices.available && !root.devices.pairing
            onClicked: root.startPairing()
        }

        Item {
            Layout.fillWidth: true
        }

        QQC2.Button {
            text: i18n("Refresh")
            icon.name: "view-refresh"
            enabled: root.devices.available
            onClicked: root.devices.refresh()
        }
    }

    function startPairing() {
        // Empty label: the daemon falls back to the name the phone reports for itself,
        // which is its model rather than a generic "Phone" for every device.
        root.devices.beginPairing("");
        pairingSheet.secondsLeft = root.devices.pairingSecondsLeft();
        pairingSheet.open();
    }

    Kirigami.PromptDialog {
        id: confirmRemoval

        // Same parenting fix as the pairing dialog.
        parent: QQC2.Overlay.overlay

        property string deviceId
        property string deviceName

        title: i18n("Remove this phone?")
        subtitle: i18n("“%1” will no longer be able to unlock this computer. You can pair it again later.", confirmRemoval.deviceName)
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                text: i18n("Remove")
                icon.name: "edit-delete"
                onTriggered: {
                    root.devices.removeDevice(confirmRemoval.deviceId);
                    confirmRemoval.close();
                }
            }
        ]
    }

    Kirigami.Dialog {
        id: pairingSheet

        // Kirigami.Dialog defaults its parent to applicationWindow().overlay, which does not
        // exist inside a KCM — it then falls back to the declaring item, so the dialog was
        // capped at the page's height rather than the window's and scrolled the QR out of
        // view. Parenting to the window overlay lets the height come from the content.
        parent: QQC2.Overlay.overlay
        // A top-level popup window is not bounded by the host window, so the QR keeps its
        // scannable size even when the module is embedded in a small one.
        popupType: QQC2.Popup.Window

        readonly property int codeSize: Kirigami.Units.gridUnit * 13
        readonly property int quietZone: Kirigami.Units.gridUnit
        readonly property int bodyWidth: Kirigami.Units.gridUnit * 20

        property int secondsLeft: 0

        title: i18n("Pair a phone")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.gridUnit
        preferredWidth: bodyWidth + (Kirigami.Units.gridUnit * 2)

        // With popupType: Window, Kirigami hides its own header and the window manager draws
        // the titlebar — but the dialog background hardcodes the View colour set, which is
        // lighter than the titlebar and leaves a visible seam. Match the Window colour set.
        background: Rectangle {
            Kirigami.Theme.colorSet: Kirigami.Theme.Window
            Kirigami.Theme.inherit: false
            color: Kirigami.Theme.backgroundColor
        }
        onRejected: root.devices.cancelPairing()
        onClosed: root.devices.cancelPairing()

        // The daemon consumes the token the moment the phone pairs, which the polling timer
        // notices within a second. Without this the dialog would sit there showing a QR that
        // has already been used.
        Connections {
            target: root.devices

            function onPairingChanged() {
                if (!root.devices.pairing && pairingSheet.opened) {
                    pairingSheet.close();
                }
            }
        }

        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing

            // Every child is pinned to the same fixed width, so no child's implicit width can
            // drive the dialog. Without this the countdown relaid the whole window out each
            // second as "9 seconds" became "10 seconds".
            QQC2.Label {
                Layout.preferredWidth: pairingSheet.bodyWidth
                Layout.maximumWidth: pairingSheet.bodyWidth
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                text: i18n("Scan this with the kpasskey app, then check the code matches.")
            }

            // The white surround is the QR quiet zone. Without it a scanner cannot find the
            // code's edges, and a themed (dark) background makes it unreadable outright.
            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                implicitWidth: pairingSheet.codeSize + (pairingSheet.quietZone * 2)
                implicitHeight: implicitWidth
                color: "white"
                radius: Kirigami.Units.smallSpacing

                Prison.Barcode {
                    anchors.centerIn: parent
                    width: pairingSheet.codeSize
                    height: pairingSheet.codeSize
                    barcodeType: Prison.Barcode.QRCode
                    content: root.devices.pairingUri
                    foregroundColor: "black"
                    backgroundColor: "white"
                }
            }

            ColumnLayout {
                Layout.preferredWidth: pairingSheet.bodyWidth
                Layout.maximumWidth: pairingSheet.bodyWidth
                spacing: 0

                QQC2.Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    opacity: 0.7
                    font: Kirigami.Theme.smallFont
                    text: i18n("Confirmation code")
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: root.devices.pairingCode
                    // Not monospaced: six digits gain nothing from fixed advance widths, and
                    // the theme's UI font at size and weight reads far better.
                    font.pointSize: Kirigami.Theme.defaultFont.pointSize + 8
                    font.bold: true
                    font.letterSpacing: Kirigami.Units.smallSpacing
                }
            }

            QQC2.Label {
                Layout.preferredWidth: pairingSheet.bodyWidth
                Layout.maximumWidth: pairingSheet.bodyWidth
                horizontalAlignment: Text.AlignHCenter
                opacity: 0.7
                font: Kirigami.Theme.smallFont
                // Kept short and on its own line so a changing number can never reflow.
                text: pairingSheet.secondsLeft > 0
                    ? i18n("Expires in %1s", pairingSheet.secondsLeft)
                    : i18n("Expired")
            }
        }
    }
}
