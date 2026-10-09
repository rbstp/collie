import QtQuick
import Quickshell
import "CollieTray"
import "CollieTray/Printable.js" as Printable

// Drives Service.qml against Linux/Tests/fake_collied.py; run-service-test.py checks both sides.
ShellRoot {
  id: harness
  property var states: []
  property var notes: ({})
  property double confirmAt: 0

  function note(key, value) {
    var n = notes
    n[key] = value
    notes = n
  }

  Service {
    id: svc
    onStateChanged: harness.states = harness.states.concat([svc.state + ":" + svc.pending])
    onPendingChanged: harness.states = harness.states.concat([svc.state + ":" + svc.pending])
    onPhaseChanged: {
      if (phase === "confirm") {
        harness.confirmAt = Date.now()
        svc.pairAnswer(true)
        harness.note("answeredBeforeArmed", svc.answered)
        armedAnswer.start()
      }
      if (phase === "invite") harness.note("qrSize", svc.qr.length)
      if (phase === "finished" && !harness.notes.firstDone) {
        harness.note("firstDone", svc.message)
        cancelTest.start()
      }
    }
  }

  Timer { id: armedAnswer; interval: 1200; onTriggered: { svc.pairAnswer(true); svc.pairAnswer(true); harness.note("answered", svc.answered) } }

  Timer {
    id: cancelTest
    interval: 300
    onTriggered: {
      svc.pairCancel()
      svc.pairStart()
      cancelDrop.start()
    }
  }
  Timer { id: cancelDrop; interval: 800; onTriggered: { harness.note("cancelPhase", svc.phase); svc.pairCancel(); harness.note("afterCancel", svc.phase) } }

  Timer { interval: 900; running: true; onTriggered: svc.pairStart() }
  Timer { interval: 5000; running: true; onTriggered: svc.refresh() }
  Timer { interval: 6000; running: true; onTriggered: { harness.note("peers", svc.peers.map(function(p) { return Printable.printable(p.label) })); svc.toggle() } }
  Timer {
    interval: 8500
    running: true
    onTriggered: {
      harness.note("lastError", svc.lastError)
      harness.note("printable", Printable.printable("ab‮cd’\u{1f600}\ń"))
      console.log("RESULT " + JSON.stringify({ states: harness.states, notes: harness.notes }))
      Qt.quit()
    }
  }
}
