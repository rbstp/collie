"""Runs Service.qml in qs against fake_collied.py and checks both sides."""
import json
import shutil
import os
import pathlib
import subprocess
import sys
import tempfile
import time

here = pathlib.Path(__file__).resolve().parent
with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    data_home, config_home = tmp / "data", tmp / "config"
    (data_home / "collie").mkdir(parents=True, mode=0o700)
    (config_home / "systemd/user").mkdir(parents=True)
    calls = tmp / "calls"
    fake_exe = tmp / "bin dir/collied"
    fake_exe.parent.mkdir()
    fake_exe.write_text(f'#!/bin/sh\necho "$1" >> "{calls}"\necho "stopped; stays off until collied start"\n')
    fake_exe.chmod(0o755)
    (config_home / "systemd/user/collied.service").write_text(
        f'[Service]\nExecStart="{fake_exe}" "run"\nEnvironment="XDG_DATA_HOME={data_home}"\n')
    config = tmp / "qs"
    shutil.copytree(here / "service", config)
    shutil.copytree(here.parent / "CollieTray", config / "CollieTray")
    sock, log = data_home / "collie/control.sock", tmp / "seen.json"
    env = dict(os.environ, HOME=str(tmp), XDG_CONFIG_HOME=str(config_home), XDG_DATA_HOME="/nonexistent")
    # collied starts after the widget, so its first connects fail and it must retry.
    shell = subprocess.Popen(["qs", "-p", config], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    time.sleep(3)
    server = subprocess.Popen([sys.executable, here / "fake_collied.py", sock, log])
    try:
        text, _ = shell.communicate(timeout=30)
    finally:
        shell.kill()
        server.kill()
    result = next((json.loads(l.split("RESULT ", 1)[1]) for l in text.splitlines() if "RESULT " in l), None)
    seen = json.loads(log.read_text()) if log.exists() else {"watch": 0, "pair": [], "lines": []}
    failures = []

    def check(ok, what):
        if not ok:
            failures.append(what)

    check(result is not None, "harness printed no RESULT:\n" + text[-3000:])
    if result:
        states, notes = result["states"], result["notes"]
        want = ["running:0", "running:2", "off:0", "outdated:0", "running:0"]
        it = iter(states)
        check(all(s in it for s in want), f"watch states {states} lack {want} in order")
        check(notes.get("answeredBeforeArmed") is False, "Pair answered before it was armed")
        check(notes.get("answered") is True, "Pair not answered once armed")
        check(notes.get("answeredBeforeClick") is False, "Pair confirmed on its own once armed")
        check(notes.get("attempts", 0) >= 2, f"no failed connect before collied came up: {notes.get('attempts')} watch attempts")
        check(notes.get("busyAfterToggle") is False, f"busy after toggle {notes.get('busyAfterToggle')!r}")
        check(notes.get("qrSize", 0) >= 21, f"QR size {notes.get('qrSize')}")
        check(notes.get("firstDone") == "paired iPhone (nPHONE)", f"pairing ended with {notes.get('firstDone')!r}")
        check(notes.get("cancelPhase") == "invite" and notes.get("afterCancel") == "", f"cancel phases {notes}")
        check(notes.get("peers") == ["a\\u{202e}b"], f"peers {notes.get('peers')}")
        check(notes.get("printable") == "ab\\u{202e}cd\u2019\U0001f600\\n\\u{301}", f"printable in qs {notes.get('printable')!r}")
        check(notes.get("lastError") == "", f"lastError {notes.get('lastError')!r}")
    check(len(seen["pair"]) == 2, f"pair connections {seen['pair']}")
    if len(seen["pair"]) == 2:
        first, second = seen["pair"]
        check(first.get("line") == '{"cmd":"confirm","accept":true}\n', f"confirm line {first}")
        check(first.get("extra") == "", f"more than one confirm line, or no EOF after pair_done: {first.get('extra')!r}")
        check(first.get("after", 0) >= 1.0, f"confirm sent {first.get('after')} s after the candidate, before Pair was armed")
        check(second.get("eof") is True, "cancel did not close the connection")
    check(set(seen["lines"]) <= {'{"cmd":"watch"}\n', '{"cmd":"pair"}\n', '{"cmd":"peers_list"}\n'}, f"unexpected requests {seen['lines']}")
    check(calls.exists() and calls.read_text().split() == ["stop"], f"collied calls {calls.read_text() if calls.exists() else None}")
    check("SECRETCODE" not in text, "the invite reached the log")
    if failures:
        print("FAIL\n- " + "\n- ".join(failures))
        sys.exit(1)
    print("service test: ok")
