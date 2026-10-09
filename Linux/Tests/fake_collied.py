"""A scripted collied control socket for Linux/Tests/service; writes what it saw as JSON."""
import json
import os
import socket
import sys
import threading
import time

path, log_path = sys.argv[1], sys.argv[2]
seen = {"watch": 0, "pairs": 0, "pair": [], "lines": []}
lock = threading.Lock()


def send(f, obj):
    f.write((json.dumps(obj) + "\n").encode())
    f.flush()


def watch(conn, f):
    with lock:
        seen["watch"] += 1
        n = seen["watch"]
    if n == 1:
        send(f, {"type": "watch", "pending_approvals": 0})
        time.sleep(0.3)
        send(f, {"type": "watch", "pending_approvals": 2})
        time.sleep(0.5)
    elif n == 2:
        send(f, {"type": "error", "message": "unknown variant `watch`"})
    else:
        send(f, {"type": "watch", "pending_approvals": 0})
        conn.settimeout(None)
        conn.recv(1)


def pair(conn, f):
    with lock:
        seen["pairs"] += 1
        n = seen["pairs"]
    entry = {}
    send(f, {"type": "invite", "uri": "collie://pair#v=2&c=SECRETCODE", "expires_in_secs": 120})
    if n == 1:
        time.sleep(1.0)
        send(f, {"type": "confirm", "device_label": "Rich\u2019s \u202eiPhone", "node_name": "phone.ts.net",
                 "stable_id": "nPHONE", "login": "me@example.com", "user_id": 7, "tls_key": "k",
                 "terminal_key": None, "replaces": False, "previous_terminal_key": None})
        sent = time.monotonic()
        conn.settimeout(5)
        line = f.readline()
        entry = {"line": line.decode(), "after": round(time.monotonic() - sent, 2)}
        send(f, {"type": "pair_done", "paired": True, "detail": "paired iPhone (nPHONE)"})
        try:
            entry["extra"] = f.read().decode()
        except OSError:
            entry["extra"] = None
    else:
        conn.settimeout(5)
        entry = {"eof": conn.recv(100) == b""}
    with lock:
        seen["pair"].append(entry)


def handle(conn):
    f = conn.makefile("rwb")
    line = f.readline().decode()
    with lock:
        seen["lines"].append(line)
    cmd = json.loads(line).get("cmd") if line else None
    try:
        if cmd == "watch":
            watch(conn, f)
        elif cmd == "pair":
            pair(conn, f)
        elif cmd == "peers_list":
            send(f, {"type": "peers", "owner_user_id": 7, "peers": [
                {"stable_id": "nPHONE", "user_id": 7, "login": "me@example.com", "label": "a\u202eb", "paired_at": 1}]})
    except OSError:
        pass
    finally:
        conn.close()
        with open(log_path, "w") as out:
            with lock:
                json.dump(seen, out)


try:
    os.unlink(path)
except FileNotFoundError:
    pass
server = socket.socket(socket.AF_UNIX)
server.bind(path)
os.chmod(path, 0o600)
server.listen(8)
while True:
    c, _ = server.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
