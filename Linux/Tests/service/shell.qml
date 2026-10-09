import QtQuick
import Quickshell
import "CollieTray"
import "CollieTray/Printable.js" as Printable

// Drives Service.qml against Linux/Tests/fake_collied.py; run-service-test.py checks both sides.
ShellRoot {
  id: harness
  property var states: []
  property var notes: ({})

  function note(key, value) {
    var n = notes
    n[key] = value
    notes = n
  }

  Service {
    id: svc
    onStateChanged: {
      harness.states = harness.states.concat([svc.state + ":" + svc.pending])
      if (state === "running" && !scenario.running && !harness.notes.started) {
        harness.note("started", true)
        scenario.start()
      }
    }
    onWatchConnChanged: if (svc.watchConn && !harness.notes.started) harness.note("attempts", (harness.notes.attempts || 0) + 1)
    onPendingChanged: harness.states = harness.states.concat([svc.state + ":" + svc.pending])
    onPhaseChanged: {
      if (phase === "confirm") {
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

  Timer { id: armedAnswer; interval: 1200; onTriggered: { harness.note("answeredBeforeClick", svc.answered); svc.pairAnswer(true); svc.pairAnswer(true); harness.note("answered", svc.answered) } }

  Timer {
    id: cancelTest
    interval: 300
    onTriggered: {
      svc.pairCancel()
      svc.pairStart()
      cancelDrop.start()
    }
  }
  Timer { id: cancelDrop; interval: 1500; onTriggered: { harness.note("cancelPhase", svc.phase); svc.pairCancel(); harness.note("afterCancel", svc.phase) } }

  // Times from the first running state: collied starts after the widget.
  property int tick: 0
  Timer {
    id: scenario
    interval: 100
    repeat: true
    onTriggered: {
      harness.tick++
      if (harness.tick === 4) svc.pairStart()
      if (harness.tick === 60) svc.refresh()
      if (harness.tick === 65) {
        harness.note("peers", svc.peers.map(function(p) { return Printable.printable(p.label) }))
        svc.lastError = "unsettled"
        svc.toggle()
      }
      if (harness.tick === 85) harness.note("busyAfterToggle", svc.busy)
      if (harness.tick === 90) harness.finish()
    }
  }
  Timer { interval: 25000; running: true; onTriggered: harness.finish() }

  function finish() {
    scenario.stop()
    {
      harness.note("lastError", svc.lastError)
      harness.note("printable", Printable.printable("ab\u202ecd\u2019\u{1f600}\n\u0301"))
      console.log("RESULT " + JSON.stringify({ states: harness.states, notes: harness.notes }))
      Qt.quit()
    }
  }
}
