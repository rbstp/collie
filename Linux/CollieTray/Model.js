// collied's control socket as CollieBar speaks it: one JSON line per message, a new
// connection per request. collied refuses unknown keys, so the lines are exact.

var WATCH = '{"cmd":"watch"}\n'
var PAIR = '{"cmd":"pair"}\n'
var PEERS = '{"cmd":"peers_list"}\n'

function confirmLine(accept) {
  return '{"cmd":"confirm","accept":' + (accept ? "true" : "false") + "}\n"
}

function parseReply(line) {
  try {
    var r = JSON.parse(line)
    if (r && typeof r.type === "string") return r
  } catch (e) {}
  return { type: "invalid" }
}

function terminalKeyChange(c) {
  if (!c.terminal_key) return "none"
  if (!c.previous_terminal_key) return "new"
  return c.terminal_key === c.previous_terminal_key ? "unchanged" : "replaces the existing one"
}

function pendingText(n) {
  return n === 1 ? "1 approval pending" : n + " approvals pending"
}

function label(state, pending) {
  if (state === "off") return "collie: off"
  if (state === "running" && pending > 0) return "collie: running, " + pendingText(pending)
  return "collie: running"
}

function absolute(dir) {
  return typeof dir === "string" && dir.charAt(0) === "/" ? dir : ""
}

function unitPath(configHome, home) {
  return (absolute(configHome) || home + "/.config") + "/systemd/user/collied.service"
}

// Undoes the quoting `collied service install` writes: \" \\ %% and, with `dollar`, the $$ of
// arguments (the executable keeps its $ as written).
function unquote(q, dollar) {
  var out = ""
  for (var i = 0; i < q.length; i++) {
    var c = q.charAt(i)
    var n = q.charAt(i + 1)
    if ((c === "\\" && (n === '"' || n === "\\")) || (c === "%" && n === "%") || (dollar && c === "$" && n === "$")) {
      out += n
      i++
    } else {
      out += c
    }
  }
  return out
}

// The first quoted word of a unit line, still quoted inside.
function firstQuoted(s) {
  if (s.charAt(0) !== '"') return null
  for (var i = 1; i < s.length; i++) {
    if (s.charAt(i) === "\\") i++
    else if (s.charAt(i) === '"') return s.slice(1, i)
  }
  return null
}

// The executable and the pinned XDG_DATA_HOME of the unit, as collied stop and start use them.
function unitInfo(text) {
  var info = { exe: "", dataHome: "" }
  var lines = String(text || "").split("\n")
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i]
    if (line.indexOf("ExecStart=") === 0) {
      var exe = firstQuoted(line.slice("ExecStart=".length))
      if (exe !== null) info.exe = absolute(unquote(exe, false))
    } else if (line.indexOf('Environment="XDG_DATA_HOME=') === 0) {
      var value = firstQuoted(line.slice("Environment=".length))
      if (value !== null) info.dataHome = absolute(unquote(value, false).slice("XDG_DATA_HOME=".length))
    }
  }
  return info
}

function dataDir(unitDataHome, envDataHome, home) {
  return (absolute(unitDataHome) || absolute(envDataHome) || home + "/.local/share") + "/collie"
}

// `qrencode -t ASCII` draws a module as "##" or two spaces: rows of "1" and "0".
function qrRows(text) {
  var rows = []
  var lines = String(text || "").split("\n")
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i].replace(/\s+$/, "")
    if (line === "" && rows.length === 0) continue
    rows.push(line)
  }
  while (rows.length > 0 && rows[rows.length - 1] === "") rows.pop()
  var size = rows.length
  var out = []
  for (var r = 0; r < size; r++) {
    var row = ""
    for (var m = 0; m < size; m++) row += rows[r].charAt(2 * m) === "#" ? "1" : "0"
    out.push(row)
  }
  return out
}

function secondsLeft(deadlineMs, nowMs) {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000))
}

if (typeof module !== "undefined")
  module.exports = {
    WATCH: WATCH,
    PAIR: PAIR,
    PEERS: PEERS,
    confirmLine: confirmLine,
    parseReply: parseReply,
    terminalKeyChange: terminalKeyChange,
    pendingText: pendingText,
    label: label,
    unitPath: unitPath,
    unquote: unquote,
    unitInfo: unitInfo,
    dataDir: dataDir,
    qrRows: qrRows,
    secondsLeft: secondsLeft,
  }
