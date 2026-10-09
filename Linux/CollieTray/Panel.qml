import QtQuick
import QtQuick.Effects
import Quickshell
import qs.Commons
import qs.Ui
import "Model.js" as Model
import "Printable.js" as Printable

Panel {
  id: root
  moduleName: "rbstp.collie"
  ipcTarget: "rbstp.collie"

  readonly property var svc: bar && bar.shell ? bar.shell.serviceFor("rbstp.collie") : null
  readonly property bool running: !!svc && svc.running
  readonly property string phase: svc ? svc.phase : ""
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  property double nowMs: Date.now()
  readonly property int secondsLeft: svc ? Model.secondsLeft(svc.deadline, nowMs) : 0

  function status() {
    if (!svc) return "collied: Off"
    if (svc.starting) return "collied: Starting…"
    return svc.running ? "collied: Running" : "collied: Off"
  }

  function escapeKey() {
    if (phase === "confirm") {
      if (!svc.answered) svc.pairAnswer(false)
    } else if (phase !== "") svc.pairCancel()
    else root.close()
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onOpenedChanged: {
    if (!svc) return
    if (opened) {
      svc.refresh()
      Qt.callLater(function() { keyCatcher.forceActiveFocus() })
    } else svc.pairCancel()
  }

  // A bar that goes away with its screen takes the pairing with it, as closing does.
  Component.onDestruction: if (opened && svc) svc.pairCancel()

  Timer {
    interval: 1000
    repeat: true
    running: root.opened && root.phase !== ""
    triggeredOnStart: true
    onTriggered: root.nowMs = Date.now()
  }

  // The SVG is the mask, the bar's colour the fill: a tint follows the theme.
  component Silhouette: Item {
    id: silhouette
    property color color: root.foreground

    Image {
      id: glyph
      anchors.fill: parent
      source: Qt.resolvedUrl("MenuIcon.svg")
      sourceSize.width: width * 2
      sourceSize.height: height * 2
      fillMode: Image.PreserveAspectFit
      visible: false
      layer.enabled: true
    }

    Rectangle {
      id: fill
      anchors.fill: parent
      color: silhouette.color
      visible: false
      layer.enabled: true
    }

    MultiEffect {
      anchors.fill: parent
      source: fill
      maskEnabled: true
      maskSource: glyph
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }
  }

  component Line: Text {
    width: parent ? parent.width : 0
    textFormat: Text.PlainText
    color: root.foreground
    font.family: root.fontFamily
    font.pixelSize: Style.font.body
    wrapMode: Text.Wrap
  }

  component Action: Button {
    width: parent ? parent.width : 0
    leftAlign: true
    foreground: root.foreground
    fontFamily: root.fontFamily
    opacity: enabled ? 1.0 : 0.4
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    dimmed: !root.running
    tooltipText: Model.label(root.svc ? root.svc.state : "off", root.svc ? root.svc.pending : 0)
    onPressed: function(buttonCode) { root.toggle() }

    iconComponent: Component {
      Item {
        Silhouette {
          anchors.fill: parent
          color: button.foreground
        }

        Rectangle {
          visible: !!root.svc && root.svc.state === "running" && root.svc.pending > 0
          width: Math.max(5, parent.width * 0.38)
          height: width
          radius: width / 2
          anchors.right: parent.right
          anchors.top: parent.top
          anchors.rightMargin: -1
          anchors.topMargin: -1
          color: root.bar ? root.bar.urgent : Color.urgent
          border.width: 1
          border.color: Color.bar.background
        }
      }
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(320))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(640))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.escapeKey()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        width: parent.width
        spacing: Style.space(10)

        PanelHero {
          width: parent.width
          title: "Collie"
          meta: root.status()
          foreground: root.foreground
          fontFamily: root.fontFamily
          iconOpacity: root.running ? 1.0 : 0.45
          iconComponent: Component {
            Silhouette {
              width: Style.font.display
              height: Style.font.display
            }
          }
        }

        // ---------- Menu ----------
        Column {
          visible: root.phase === ""
          width: parent.width
          spacing: Style.space(6)

          Line {
            visible: !!root.svc && root.svc.state === "outdated"
            text: "Update collied (just collied-install)"
          }
          Line {
            visible: !!root.svc && root.svc.state === "running" && root.svc.pending > 0
            text: root.svc ? Model.pendingText(root.svc.pending) : ""
          }
          Line {
            visible: !!root.svc && root.svc.lastError !== ""
            text: root.svc ? root.svc.lastError : ""
            color: root.bar ? root.bar.urgent : Color.urgent
          }

          Action {
            text: root.running ? "Turn Off" : "Turn On"
            enabled: !!root.svc && !root.svc.busy && !root.svc.starting
            onClicked: root.svc.toggle()
          }
          Action {
            text: "Pair a Phone…"
            enabled: root.running
            onClicked: root.svc.pairStart()
          }

          PanelSeparator { width: parent.width; foreground: root.foreground }
          PanelSectionHeader { text: "Paired phones"; foreground: root.foreground; fontFamily: root.fontFamily }
          Line {
            visible: !root.running || root.svc.peers.length === 0
            text: root.running ? "None" : "collied is off"
            color: root.dim
          }
          Repeater {
            model: root.running ? root.svc.peers : []
            Line {
              required property var modelData
              text: Printable.printable(modelData.label) + " (" + Printable.printable(modelData.login) + ")"
            }
          }

          PanelSeparator { width: parent.width; foreground: root.foreground }
          Action {
            text: "Open Audit Log"
            enabled: !!root.svc && root.svc.auditLogExists
            onClicked: {
              root.svc.openAuditLog()
              root.close()
            }
          }
          Action {
            text: "Quit (stops collied)"
            enabled: !!root.svc && !root.svc.busy
            onClicked: root.svc.quit()
          }
        }

        // ---------- Pairing ----------
        Line {
          visible: root.phase === "connecting"
          text: "Opening a pairing window…"
          color: root.dim
        }

        Column {
          visible: root.phase === "invite"
          width: parent.width
          spacing: Style.space(10)

          Line { text: "Scan with the Collie app"; font.bold: true; horizontalAlignment: Text.AlignHCenter }

          // One native rectangle per module, with the quiet zone as padding: crisp, and the
          // code never touches a file.
          Rectangle {
            id: qrCanvas
            readonly property int size: root.svc ? root.svc.qr.length : 0
            readonly property int module: size > 0 ? Math.max(3, Math.floor(Style.space(220) / (size + 8))) : 0
            anchors.horizontalCenter: parent.horizontalCenter
            width: (size + 8) * module
            height: width
            color: "white"
            radius: Style.cornerRadius

            Grid {
              x: 4 * qrCanvas.module
              y: 4 * qrCanvas.module
              columns: qrCanvas.size

              Repeater {
                model: qrCanvas.size * qrCanvas.size
                Rectangle {
                  required property int index
                  width: qrCanvas.module
                  height: qrCanvas.module
                  color: root.svc.qr[Math.floor(index / qrCanvas.size)].charAt(index % qrCanvas.size) === "1" ? "black" : "white"
                }
              }
            }
          }

          Line {
            text: "Waiting up to " + root.secondsLeft + " s for the phone"
            color: root.dim
            horizontalAlignment: Text.AlignHCenter
          }
          Action {
            text: "Cancel"
            onClicked: root.svc.pairCancel()
          }
        }

        Column {
          visible: root.phase === "confirm" && !!root.svc && !!root.svc.candidate
          width: parent.width
          spacing: Style.space(6)

          readonly property var c: root.svc && root.svc.candidate ? root.svc.candidate : ({})

          Line { text: "A phone presented the pairing code:" }
          Line { text: "device: " + Printable.printable(parent.c.device_label || "") }
          Line { text: "node: " + Printable.printable(parent.c.node_name || "") + " (" + Printable.printable(parent.c.stable_id || "") + ")" }
          Line { text: "user: " + Printable.printable(parent.c.login || "") + " (" + String(parent.c.user_id) + ")" }
          Line { text: "terminal key: " + Model.terminalKeyChange(parent.c) }
          Line {
            visible: !!parent.c.replaces
            text: "replaces an existing pairing of this node"
          }
          Line { text: "Pair this phone? (" + root.secondsLeft + " s)"; font.bold: true }

          Row {
            width: parent.width
            spacing: Style.space(8)

            Button {
              width: (parent.width - parent.spacing) / 2
              text: "Don't Pair"
              bordered: true
              foreground: root.foreground
              fontFamily: root.fontFamily
              enabled: !!root.svc && !root.svc.answered
              opacity: enabled ? 1.0 : 0.4
              onClicked: root.svc.pairAnswer(false)
            }
            Button {
              width: (parent.width - parent.spacing) / 2
              text: "Pair"
              bordered: true
              foreground: root.foreground
              fontFamily: root.fontFamily
              enabled: !!root.svc && root.svc.armed && !root.svc.answered && root.secondsLeft > 0
              opacity: enabled ? 1.0 : 0.4
              onClicked: root.svc.pairAnswer(true)
            }
          }
        }

        Column {
          visible: root.phase === "finished"
          width: parent.width
          spacing: Style.space(10)

          Line { text: root.svc ? root.svc.message : "" }
          Action {
            text: "OK"
            onClicked: root.svc.pairCancel()
          }
        }
      }
    }
  }
}
