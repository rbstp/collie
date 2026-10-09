// node --test Linux/Tests: the wire format and the parsing the bar widget relies on.
const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const M = require("../CollieTray/Model.js");
const { printable } = require("../CollieTray/Printable.js");

test("request lines are the ones collied and CollieBar use", () => {
  assert.equal(M.WATCH, '{"cmd":"watch"}\n');
  assert.equal(M.PAIR, '{"cmd":"pair"}\n');
  assert.equal(M.PEERS, '{"cmd":"peers_list"}\n');
  assert.equal(M.confirmLine(true), '{"cmd":"confirm","accept":true}\n');
  assert.equal(M.confirmLine(false), '{"cmd":"confirm","accept":false}\n');
  assert.equal(M.confirmLine("yes"), '{"cmd":"confirm","accept":true}\n');
});

test("replies decode like CollieBar's fixtures", () => {
  assert.deepEqual(M.parseReply('{"type":"invite","uri":"collie://pair#v=1","expires_in_secs":120}'), {
    type: "invite",
    uri: "collie://pair#v=1",
    expires_in_secs: 120,
  });
  assert.equal(M.parseReply('{"type":"pair_done","paired":false,"detail":"not confirmed on the machine"}').paired, false);
  assert.equal(M.parseReply('{"type":"error","message":"a pairing window is already open"}').type, "error");
  assert.deepEqual(M.parseReply('{"type":"peers","owner_user_id":null,"peers":[]}').peers, []);
  assert.equal(M.parseReply('{"type":"watch","pending_approvals":3}').pending_approvals, 3);
  assert.equal(M.parseReply("{not json").type, "invalid");
  assert.equal(M.parseReply('{"no":"type"}').type, "invalid");
  assert.equal(M.parseReply("null").type, "invalid");
});

test("terminal key change matches collied", () => {
  const c = (terminal_key, previous_terminal_key) => M.terminalKeyChange({ terminal_key, previous_terminal_key });
  assert.equal(c(null, null), "none");
  assert.equal(c("k", null), "new");
  assert.equal(c("k", "k"), "unchanged");
  assert.equal(c("k", "j"), "replaces the existing one");
});

test("printable escapes what the phone could hide, like CollieBar", () => {
  assert.equal(printable('Rich’s "iPhone" \\ 15'), 'Rich’s "iPhone" \\ 15');
  assert.equal(printable("ab‮cd"), "ab\\u{202e}cd");
  assert.equal(printable("a​b⁦"), "a\\u{200b}b\\u{2066}");
  assert.equal(printable("a\nb\tc\r"), "a\\nb\\tc\\r");
  assert.equal(printable("\u001b[31mred"), "\\u{1b}[31mred");
  assert.equal(printable("x\u007f "), "x\\u{7f}\\u{2028}");
  assert.equal(printable("a b́c　d️eㅤ"), "a\\u{a0}b\\u{301}c\\u{3000}d\\u{fe0f}e\\u{3164}");
  assert.equal(printable("\0"), "\\0");
  assert.equal(printable("\ud800x\u{e0001}\u{10ffff}"), "\\u{d800}x\\u{e0001}\\u{10ffff}");
  assert.equal(printable("\u{1f600} café 日本"), "\u{1f600} café 日本");
});

// What `collied service install` writes (crates/collied/src/service/systemd.rs).
const UNIT = `# Written by \`collied service install\`; rewritten on every install.
[Service]
Type=exec
ExecStart="/home/me/50%% off/.cargo/bin/collied" "--config" "/home/me/a b\\"$$HOME%%h\\\\;.toml" "run"
Environment="XDG_DATA_HOME=/home/me/$data%%"
Environment="XDG_CACHE_HOME=/home/me/.cache"
`;

test("the unit gives the executable and the data dir", () => {
  assert.deepEqual(M.unitInfo(UNIT), { exe: "/home/me/50% off/.cargo/bin/collied", dataHome: "/home/me/$data%" });
  assert.deepEqual(M.unitInfo(""), { exe: "", dataHome: "" });
  assert.deepEqual(M.unitInfo('ExecStart="relative/collied" "run"\nEnvironment="XDG_DATA_HOME=rel"'), { exe: "", dataHome: "" });
  assert.equal(M.unquote('a\\"b\\\\c%%d$$e', true), 'a"b\\c%d$e');
  assert.equal(M.unquote("$$", false), "$$");
});

test("paths follow collied's XDG rules", () => {
  assert.equal(M.unitPath("", "/h"), "/h/.config/systemd/user/collied.service");
  assert.equal(M.unitPath("rel", "/h"), "/h/.config/systemd/user/collied.service");
  assert.equal(M.unitPath("/x", "/h"), "/x/systemd/user/collied.service");
  assert.equal(M.dataDir("/unit", "/env", "/h"), "/unit/collie");
  assert.equal(M.dataDir("", "/env", "/h"), "/env/collie");
  assert.equal(M.dataDir("", "rel", "/h"), "/h/.local/share/collie");
});

test("qrencode's matrix becomes rows of modules", () => {
  const ascii = execFileSync("qrencode", ["-l", "M", "-t", "ASCII", "-m", "0"], { input: "collie://pair#v=2&c=x" }).toString();
  const rows = M.qrRows(ascii);
  assert.ok(rows.length >= 21 && rows.length % 4 === 1, `size ${rows.length}`);
  for (const row of rows) assert.match(row, new RegExp(`^[01]{${rows.length}}$`));
  // Finder pattern corners.
  assert.equal(rows[0].slice(0, 7), "1111111");
  assert.equal(rows[1].slice(0, 7), "1000001");
  assert.equal(rows[rows.length - 1].slice(0, 7), "1111111");
  assert.deepEqual(M.qrRows(""), []);
});

test("labels and countdowns", () => {
  assert.equal(M.label("off", 0), "collie: off");
  assert.equal(M.label("outdated", 0), "collie: running");
  assert.equal(M.label("running", 1), "collie: running, 1 approval pending");
  assert.equal(M.label("running", 2), "collie: running, 2 approvals pending");
  assert.equal(M.secondsLeft(10500, 0), 11);
  assert.equal(M.secondsLeft(0, 5), 0);
});
