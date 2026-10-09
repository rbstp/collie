import QtQuick
import Quickshell
import Quickshell.Io
import "Model.js" as Model
import "Printable.js" as Printable

// One collied connection for every bar: the watch stream, the commands and a pairing.
// Never log a pairing line: the invite carries the one-time code.
Item {
  id: root

  property var shell: null
  property var manifest: null

  readonly property string home: Quickshell.env("HOME")
  readonly property var unit: Model.unitInfo(unitFile.text())
  readonly property string dataDir: Model.dataDir(unit.dataHome, Quickshell.env("XDG_DATA_HOME"), home)
  readonly property string socketPath: dataDir + "/control.sock"
  readonly property string auditLog: dataDir + "/audit.log"

  property string state: "off"
  property int pending: 0
  readonly property bool running: state !== "off"
  property bool starting: false
  property bool busy: false
  property string lastError: ""
  property var peers: []
  property bool auditLogExists: false

  // "" | "connecting" | "invite" | "confirm" | "finished"
  property string phase: ""
  property var qr: []
  property double deadline: 0
  property var candidate: null
  property bool armed: false
  property bool answered: false
  property string message: ""
  property var pairConn: null

  function apply(state, pending) {
    if (state !== "off") starting = false
    root.state = state
    root.pending = pending
    if (state === "off") peers = []
  }

  function refresh() {
    auditProbe.running = true
    if (!running) return
    var s = lineSocket.createObject(root, { path: socketPath })
    var done = false
    function finish() {
      if (done) return
      done = true
      s.destroy()
    }
    s.lineReceived.connect(function(line) {
      var r = Model.parseReply(line)
      if (r.type === "peers" && Array.isArray(r.peers)) peers = r.peers
      finish()
    })
    s.connectionStateChanged.connect(function() {
      if (s.connected) {
        s.write(Model.PEERS)
        s.flush()
      } else finish()
    })
    s.error.connect(finish)
    s.connected = true
  }

  function collied(command, then) {
    if (busy) return
    if (unit.exe === "") {
      lastError = "not installed: run collied service install"
      return
    }
    busy = true
    colliedProc.then = then || null
    colliedProc.command = [unit.exe, command]
    colliedProc.running = true
  }

  function toggle() {
    var on = !running
    collied(on ? "start" : "stop", function(ok) {
      if (on && ok && !running) {
        starting = true
        startingTimer.restart()
      }
    })
  }

  // The shell stays up: Quit stops collied and takes the icon off the bar.
  function quit() {
    collied("stop", function() {
      Quickshell.execDetached(["omarchy-plugin-disable", "rbstp.collie"])
    })
  }

  function openAuditLog() {
    Quickshell.execDetached(["omarchy-launch-tui", "--app-id=org.omarchy.collie-audit", "env", "LESSSECURE=1", "less", "+G", "--", auditLog])
  }

  function pairStart() {
    if (pairConn) return
    qr = []
    candidate = null
    message = ""
    phase = "connecting"
    var s = lineSocket.createObject(root, { path: socketPath })
    pairConn = s
    s.lineReceived.connect(function(line) {
      if (pairConn === s) pairLine(Model.parseReply(line))
    })
    s.connectionStateChanged.connect(function() {
      if (pairConn !== s) return
      if (s.connected) {
        s.write(Model.PAIR)
        s.flush()
      } else pairFinish("collied closed the pairing")
    })
    s.error.connect(function() {
      if (pairConn === s && !s.connected) pairFinish("collied is not running")
    })
    s.connected = true
  }

  function pairLine(r) {
    if (r.type === "invite" && typeof r.uri === "string") {
      deadline = Date.now() + (r.expires_in_secs || 120) * 1000
      qrProc.uri = r.uri
      qrProc.running = true
    } else if (r.type === "confirm" && (phase === "invite" || phase === "connecting")) {
      candidate = r
      armed = false
      answered = false
      deadline = Date.now() + 60000
      phase = "confirm"
      armTimer.restart()
    } else if (r.type === "pair_done") {
      pairFinish(Printable.printable(r.detail || ""))
      refresh()
    } else if (r.type === "error") {
      pairFinish(Printable.printable(r.message || ""))
    } else {
      pairFinish("unexpected reply from collied")
    }
  }

  function pairAnswer(accept) {
    if (phase !== "confirm" || answered || !pairConn) return
    if (accept && (!armed || Model.secondsLeft(deadline, Date.now()) === 0)) return
    answered = true
    pairConn.write(Model.confirmLine(accept))
    pairConn.flush()
  }

  function dropPairConn() {
    var s = pairConn
    pairConn = null
    qrProc.uri = ""
    if (qrProc.running) qrProc.running = false
    if (s) s.destroy()
  }

  function pairFinish(text) {
    dropPairConn()
    qr = []
    candidate = null
    message = text
    phase = "finished"
  }

  // collied takes the closed connection as a cancel, or as Don't Pair.
  function pairCancel() {
    dropPairConn()
    qr = []
    candidate = null
    message = ""
    phase = ""
  }

  Component {
    id: lineSocket

    Socket {
      id: sock
      signal lineReceived(string line)
      parser: SplitParser {
        onRead: function(line) { sock.lineReceived(line) }
      }
    }
  }

  FileView {
    id: unitFile
    path: Model.unitPath(Quickshell.env("XDG_CONFIG_HOME"), root.home)
    blockLoading: true
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
  }

  // An open watch connection is a running collied; one older than watch answers an error.
  // A Socket never connects again after a failed attempt, so each attempt gets a new one.
  property var watchConn: null
  property bool outdated: false

  function watch() {
    if (watchConn) return
    var s = lineSocket.createObject(root, { path: socketPath })
    watchConn = s
    function drop() {
      if (watchConn !== s) return
      watchConn = null
      s.destroy()
      if (!outdated) apply("off", 0)
      retryTimer.restart()
    }
    s.lineReceived.connect(function(line) {
      if (watchConn !== s) return
      var r = Model.parseReply(line)
      if (r.type === "watch") {
        outdated = false
        apply("running", Number(r.pending_approvals) || 0)
      } else if (r.type === "error") {
        outdated = true
        apply("outdated", 0)
      }
    })
    s.connectionStateChanged.connect(function() {
      if (s.connected) {
        s.write(Model.WATCH)
        s.flush()
      } else drop()
    })
    s.error.connect(function() {
      if (!s.connected) {
        outdated = false
        drop()
      }
    })
    s.connected = true
  }

  Component.onCompleted: watch()

  onSocketPathChanged: {
    if (!watchConn) return
    var s = watchConn
    watchConn = null
    s.destroy()
    watch()
  }

  Timer {
    id: retryTimer
    interval: 2000
    onTriggered: root.watch()
  }

  // The tailnet can take up to a minute to come up.
  Timer {
    id: startingTimer
    interval: 90000
    onTriggered: root.starting = false
  }

  // Pair goes live a second after the candidate shows, so a click meant for something
  // else cannot confirm it.
  Timer {
    id: armTimer
    interval: 1000
    onTriggered: root.armed = true
  }

  Process {
    id: auditProbe
    command: ["test", "-f", root.auditLog]
    onExited: function(code) { root.auditLogExists = code === 0 }
  }

  Process {
    id: colliedProc
    property var then: null
    property int code: -1
    property int pendingStreams: 0

    function settle() {
      if (running || pendingStreams > 0) return
      var out = (String(output.text || "") + String(errors.text || "")).trim()
      var ok = code === 0
      root.busy = false
      root.lastError = ok && out.indexOf("already") !== 0 ? "" : Printable.printable(out)
      var then = colliedProc.then
      colliedProc.then = null
      if (then) then(ok)
    }

    stdout: StdioCollector {
      id: output
      waitForEnd: true
      onStreamFinished: {
        colliedProc.pendingStreams--
        colliedProc.settle()
      }
    }
    stderr: StdioCollector {
      id: errors
      waitForEnd: true
      onStreamFinished: {
        colliedProc.pendingStreams--
        colliedProc.settle()
      }
    }
    onStarted: pendingStreams = 2
    onExited: function(exitCode) {
      code = exitCode
      Qt.callLater(settle)
    }
  }

  // The URI goes to qrencode on stdin only: argv shows in ps.
  Process {
    id: qrProc
    property string uri: ""
    command: ["qrencode", "-l", "M", "-t", "ASCII", "-m", "0"]
    stdinEnabled: true
    onStarted: {
      write(uri)
      uri = ""
      stdinEnabled = false
    }
    stdout: StdioCollector {
      id: qrOut
      waitForEnd: true
      onStreamFinished: {
        var rows = Model.qrRows(text)
        if (!root.pairConn || root.phase !== "connecting") return
        if (rows.length === 0) root.pairFinish("cannot draw the pairing QR")
        else {
          root.qr = rows
          root.phase = "invite"
        }
      }
    }
    onExited: function(exitCode) {
      stdinEnabled = true
      if (exitCode !== 0 && root.pairConn && root.phase === "connecting") root.pairFinish("cannot draw the pairing QR")
    }
  }
}
