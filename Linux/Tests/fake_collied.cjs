const fs = require("node:fs");
const net = require("node:net");

const [socketPath, logPath] = process.argv.slice(2);
const seen = { watch: 0, pairs: 0, pair: [], lines: [] };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function* lines(socket) {
  let buffer = "";
  for await (const chunk of socket) {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf("\n")) >= 0) {
      yield buffer.slice(0, end + 1);
      buffer = buffer.slice(end + 1);
    }
  }
  if (buffer) yield buffer;
}

async function rest(reader) {
  let data = "";
  for await (const line of reader) data += line;
  return data;
}

function within(promise, milliseconds) {
  let timer;
  const expired = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error("socket timeout")), milliseconds);
  });
  return Promise.race([promise, expired]).finally(() => clearTimeout(timer));
}

function save() {
  fs.writeFileSync(logPath, JSON.stringify(seen));
}

function send(socket, value) {
  socket.write(`${JSON.stringify(value)}\n`);
}

async function watch(socket, reader) {
  const number = ++seen.watch;
  save();
  if (number === 1) {
    send(socket, { type: "watch", pending_approvals: 0 });
    await sleep(300);
    send(socket, { type: "watch", pending_approvals: 2 });
    await sleep(500);
  } else if (number === 2) {
    send(socket, { type: "error", message: "unknown variant `watch`" });
  } else {
    send(socket, { type: "watch", pending_approvals: 0 });
    await rest(reader);
  }
}

async function pair(socket, reader) {
  const number = ++seen.pairs;
  save();
  send(socket, { type: "invite", uri: "collie://pair#v=2&c=SECRETCODE", expires_in_secs: 120 });
  let entry;
  if (number === 1) {
    await sleep(1000);
    send(socket, {
      type: "confirm",
      device_label: "Rich’s ‮iPhone",
      node_name: "phone.ts.net",
      stable_id: "nPHONE",
      login: "me@example.com",
      user_id: 7,
      tls_key: "k",
      terminal_key: null,
      replaces: false,
      previous_terminal_key: null,
    });
    const sent = performance.now();
    const answer = await within(reader.next(), 5000);
    const line = answer.done ? "" : answer.value;
    entry = { line, after: Math.round((performance.now() - sent) / 10) / 100 };
    send(socket, { type: "pair_done", paired: true, detail: "paired iPhone (nPHONE)" });
    try {
      entry.extra = await within(rest(reader), 5000);
    } catch {
      entry.extra = null;
    }
  } else {
    entry = { eof: (await within(rest(reader), 5000)) === "" };
  }
  seen.pair.push(entry);
  save();
}

async function handle(socket) {
  const reader = lines(socket);
  const first = await reader.next();
  const line = first.done ? "" : first.value;
  seen.lines.push(line);
  save();
  const command = line ? JSON.parse(line).cmd : null;
  try {
    if (command === "watch") await watch(socket, reader);
    else if (command === "pair") await pair(socket, reader);
    else if (command === "peers_list") {
      send(socket, {
        type: "peers",
        owner_user_id: 7,
        peers: [{ stable_id: "nPHONE", user_id: 7, login: "me@example.com", label: "a‮b", paired_at: 1 }],
      });
    }
  } finally {
    socket.end();
    save();
  }
}

fs.rmSync(socketPath, { force: true });
const server = net.createServer((socket) => {
  handle(socket).catch((error) => {
    if (!socket.destroyed) socket.destroy();
    if (!error.code && error.message !== "socket timeout") throw error;
  });
});
server.listen(socketPath, () => fs.chmodSync(socketPath, 0o600));
