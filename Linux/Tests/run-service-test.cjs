const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function collect(child, timeout) {
  return new Promise((resolve, reject) => {
    let text = "";
    let timer;
    const finish = (then, value) => {
      clearTimeout(timer);
      then(value);
    };
    child.stdout.on("data", (data) => (text += data));
    child.stderr.on("data", (data) => (text += data));
    child.once("error", (error) => finish(reject, error));
    child.once("close", () => finish(resolve, text));
    timer = setTimeout(() => reject(new Error(`qs timed out after ${timeout / 1000} seconds`)), timeout);
  });
}

function check(failures, ok, message) {
  if (!ok) failures.push(message);
}

async function run() {
  const here = __dirname;
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "collie-service-test-"));
  let shell;
  let server;
  try {
    const dataHome = path.join(temporary, "data");
    const configHome = path.join(temporary, "config");
    fs.mkdirSync(path.join(dataHome, "collie"), { recursive: true, mode: 0o700 });
    fs.mkdirSync(path.join(configHome, "systemd/user"), { recursive: true });
    const calls = path.join(temporary, "calls");
    const fakeExecutable = path.join(temporary, "bin dir/collied");
    fs.mkdirSync(path.dirname(fakeExecutable));
    fs.writeFileSync(
      fakeExecutable,
      `#!/bin/sh\necho "$1" >> "${calls}"\necho "stopped; stays off until collied start"\n`,
    );
    fs.chmodSync(fakeExecutable, 0o755);
    fs.writeFileSync(
      path.join(configHome, "systemd/user/collied.service"),
      `[Service]\nExecStart="${fakeExecutable}" "run"\nEnvironment="XDG_DATA_HOME=${dataHome}"\n`,
    );
    const config = path.join(temporary, "qs");
    fs.cpSync(path.join(here, "service"), config, { recursive: true });
    fs.cpSync(path.join(here, "../CollieTray"), path.join(config, "CollieTray"), { recursive: true });
    const socketPath = path.join(dataHome, "collie/control.sock");
    const log = path.join(temporary, "seen.json");
    const env = { ...process.env, HOME: temporary, XDG_CONFIG_HOME: configHome, XDG_DATA_HOME: "/nonexistent" };
    shell = spawn("qs", ["-p", config], { env, stdio: ["ignore", "pipe", "pipe"] });
    const output = collect(shell, 30000);
    await sleep(3000);
    server = spawn(process.execPath, [path.join(here, "fake_collied.cjs"), socketPath, log], { stdio: "inherit" });
    const text = await output;
    const resultLine = text.split("\n").find((line) => line.includes("RESULT "));
    const result = resultLine ? JSON.parse(resultLine.split("RESULT ", 2)[1]) : null;
    const seen = fs.existsSync(log)
      ? JSON.parse(fs.readFileSync(log, "utf8"))
      : { watch: 0, pair: [], lines: [] };
    const failures = [];

    check(failures, result !== null, `harness printed no RESULT:\n${text.slice(-3000)}`);
    if (result) {
      const { states, notes } = result;
      const wanted = ["running:0", "running:2", "off:0", "outdated:0", "running:0"];
      let position = 0;
      for (const state of states) {
        if (state === wanted[position]) position++;
      }
      check(failures, position === wanted.length, `watch states ${JSON.stringify(states)} lack ${JSON.stringify(wanted)} in order`);
      check(failures, notes.answeredBeforeArmed === false, "Pair answered before it was armed");
      check(failures, notes.answered === true, "Pair not answered once armed");
      check(failures, notes.answeredBeforeClick === false, "Pair confirmed on its own once armed");
      check(failures, (notes.attempts || 0) >= 2, `no failed connect before collied came up: ${notes.attempts} watch attempts`);
      check(failures, notes.busyAfterToggle === false, `busy after toggle ${JSON.stringify(notes.busyAfterToggle)}`);
      check(failures, (notes.qrSize || 0) >= 21, `QR size ${notes.qrSize}`);
      check(failures, notes.firstDone === "paired iPhone (nPHONE)", `pairing ended with ${JSON.stringify(notes.firstDone)}`);
      check(
        failures,
        notes.cancelPhase === "invite" && notes.afterCancel === "",
        `cancel phases ${JSON.stringify(notes)}`,
      );
      check(failures, JSON.stringify(notes.peers) === JSON.stringify(["a\\u{202e}b"]), `peers ${JSON.stringify(notes.peers)}`);
      check(
        failures,
        notes.printable === "ab\\u{202e}cd’😀\\n\\u{301}",
        `printable in qs ${JSON.stringify(notes.printable)}`,
      );
      check(failures, notes.lastError === "", `lastError ${JSON.stringify(notes.lastError)}`);
    }
    check(failures, seen.pair.length === 2, `pair connections ${JSON.stringify(seen.pair)}`);
    if (seen.pair.length === 2) {
      const [first, second] = seen.pair;
      check(failures, first.line === '{"cmd":"confirm","accept":true}\n', `confirm line ${JSON.stringify(first)}`);
      check(failures, first.extra === "", `more than one confirm line, or no EOF after pair_done: ${JSON.stringify(first.extra)}`);
      check(failures, (first.after || 0) >= 1.0, `confirm sent ${first.after} s after the candidate, before Pair was armed`);
      check(failures, second.eof === true, "cancel did not close the connection");
    }
    const requests = new Set(['{"cmd":"watch"}\n', '{"cmd":"pair"}\n', '{"cmd":"peers_list"}\n']);
    check(failures, seen.lines.every((line) => requests.has(line)), `unexpected requests ${JSON.stringify(seen.lines)}`);
    const callText = fs.existsSync(calls) ? fs.readFileSync(calls, "utf8") : null;
    check(failures, callText !== null && callText.trim().split(/\s+/).join(" ") === "stop", `collied calls ${callText}`);
    check(failures, !text.includes("SECRETCODE"), "the invite reached the log");
    if (failures.length) {
      console.error(`FAIL\n- ${failures.join("\n- ")}`);
      process.exitCode = 1;
    } else {
      console.log("service test: ok");
    }
  } finally {
    if (shell) shell.kill();
    if (server) server.kill();
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
