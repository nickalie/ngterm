#!/usr/bin/env node
// Runs close-ended-remote-sessions.sh against a stub ngtermctl that answers from a fixed set of windows, trees and
// screens, and checks which rows it closes. Needs bash and jq: node --test close-ended-remote-sessions.test.js
"use strict";

const test = require("node:test");
const assert = require("node:assert");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const SCRIPT = path.join(__dirname, "close-ended-remote-sessions.sh");
const PROMPT = "Process exited. Press any key to close the terminal.";
const held = (name, pane = "left") => `last output\nngterm: ${name} (${pane}) on box disconnected, exit 1\n\n${PROMPT}`;

// The stub: answers `window list`, `tree`, `session text`, `session close` and `session hud` from FIXTURE and logs
// every call's arguments to LOG, one JSON array per line.
const STUB = `#!/usr/bin/env node
const fs = require("fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.LOG, JSON.stringify(args) + "\\n");
const fx = JSON.parse(fs.readFileSync(process.env.FIXTURE, "utf8"));
const opt = (name) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const all = (name) => args.flatMap((a, i) => (a === name ? [args[i + 1]] : []));
const reply = (result) => { process.stdout.write(JSON.stringify({ ok: true, result }) + "\\n"); };
const cmd = args.filter((a) => !a.startsWith("--")).slice(0, 2).join(" ");
if (cmd.startsWith("window list")) reply({ windows: fx.windows });
else if (cmd.startsWith("tree")) reply({ tree: fx.trees[opt("--window")] });
else if (cmd === "session text") {
  const text = fx.texts[opt("--target") + ":" + opt("--pane")];
  if (text === undefined) { process.stderr.write("session not realized\\n"); process.exit(1); }
  reply({ text });
} else if (cmd === "session close") {
  // as the app does: every id it closed, a parent's companions included
  const targets = all("--target");
  const rows = fx.trees[opt("--window")].workspaces.flatMap((w) => w.sessions);
  reply({ closed: rows.filter((r) => targets.includes(r.id) || targets.includes(r.parent)).map((r) => r.id) });
}
else if (cmd === "session hud") reply({ id: "hud" });
else { process.stderr.write("unexpected: " + args.join(" ") + "\\n"); process.exit(3); }
`;

const remote = (id, extra = {}) => ({
  id,
  name: `row ${id}`,
  remoteHost: "box",
  presentation: { state: "failed", mode: "mirror", error: "exit 1" },
  surfaces: [{ kind: "left" }],
  ...extra,
});

const FIXTURE = {
  windows: [
    { id: "W1", open: true },
    { id: "W2", open: true },
    { id: "W3", open: false },
  ],
  trees: {
    W1: {
      workspaces: [
        {
          sessions: [
            remote("A"),
            // the stream is up: the session still exists on its machine, though this pane stopped
            remote("B", { presentation: { state: "connected", mode: "mirror" } }),
            // ssh still runs in the pane
            remote("C", { foreground: ["ssh", "-tt", "box"] }),
            // reconnecting after a dropped link
            remote("D", { surfaces: [{ kind: "left", reconnect: { failures: 2 } }] }),
            // a parent that goes, with an ended companion and a page companion
            remote("P"),
            remote("E", { parent: "P" }),
            { id: "G", name: "page G", parent: "P", content: "page", surfaces: [] },
            // a parent that stays, since a companion of it is still running
            remote("Q"),
            remote("H", { parent: "Q", presentation: { state: "connected", mode: "presenter" } }),
            // a local session never counts, whatever its screen says
            { id: "R", name: "local R", surfaces: [{ kind: "left" }] },
            // a split whose second pane still runs something
            remote("S", { hasSplit: true }),
            // its disconnect line wrapped by a narrow pane, its stream stopped as its machine no longer has the session
            remote("T", { presentation: { state: "ended", mode: "mirror" } }),
            // not realized: its screen cannot be read
            remote("U"),
            // a local scratch shell, hidden or not, and a page over the row are this machine's work
            remote("X", { surfaces: [{ kind: "left" }, { kind: "scratch", visible: false }] }),
            remote("Y", { overlay: true, htmlOverlays: [{ id: "page" }] }),
            // an origin too old to stream: nothing tells a live session from an ended one
            remote("Z", { presentation: { state: "unsupported", mode: "mirror" } }),
            // an ended companion under a parent still running closes on its own
            remote("L", { presentation: { state: "connected", mode: "presenter" }, foreground: ["ssh", "-tt", "box"] }),
            remote("K", { parent: "L" }),
          ],
        },
      ],
    },
    W2: { workspaces: [{ sessions: [remote("V", { presentation: { state: "connecting", mode: "mirror" } })] }] },
  },
  texts: {
    "A:left": held("row A"),
    "B:left": held("row B"),
    "C:left": "$ ",
    "D:left": "Connection to box lost · reconnecting… · any key retries now",
    "P:left": held("row P"),
    "E:left": held("row E"),
    "Q:left": held("row Q"),
    "H:left": "working",
    "R:left": held("local R"),
    "S:left": held("row S"),
    "S:right": "$ make test",
    "T:left": `ngterm: a very long session name that the pane wr\napped (left) on box disconnected, exit 0\n\n${PROMPT}`,
    "V:left": held("row V"),
    "X:left": held("row X"),
    "Y:left": held("row Y"),
    "Z:left": held("row Z"),
    "K:left": held("row K"),
  },
};

function run(...args) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "close-ended-"));
  try {
    const stub = path.join(dir, "ngtermctl");
    fs.writeFileSync(stub, STUB, { mode: 0o755 });
    fs.writeFileSync(path.join(dir, "fixture.json"), JSON.stringify(FIXTURE));
    const log = path.join(dir, "log");
    fs.writeFileSync(log, "");
    const env = { ...process.env, NGTERMCTL: stub, FIXTURE: path.join(dir, "fixture.json"), LOG: log };
    for (const name of ["AGT_SOCKET", "NGTERM_SOCKET", "AGT_WINDOW_ID"]) delete env[name];
    const stdout = execFileSync(SCRIPT, args, { env, encoding: "utf8" });
    const calls = fs.readFileSync(log, "utf8").trim().split("\n").map((line) => JSON.parse(line));
    return { stdout, closes: calls.filter((c) => c[0] === "session" && c[1] === "close"), calls };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// one run serves the tests that read it: each stub call starts a node process
let full;
const fullRun = () => (full ??= run());

test("closes ended rows window by window, a parent's companions with it", () => {
  const { stdout, closes } = fullRun();
  assert.deepStrictEqual(closes, [
    ["session", "close", "--target", "A", "--target", "P", "--target", "T", "--target", "K", "--window", "W1", "--json"],
    ["session", "close", "--target", "V", "--undoable", "--window", "W2", "--json"],
  ]);
  assert.match(stdout, /^closing row A \(box\)$/m);
  assert.match(stdout, /^ {2}with it: row E$/m);
  assert.match(stdout, /^ {2}with it: page G$/m);
  assert.match(stdout, /^Closed 7 ended remote sessions$/m);
});

test("leaves running, reconnecting, connected, local and unreadable rows alone", () => {
  const { stdout } = fullRun();
  for (const id of ["B", "C", "D", "Q", "H", "R", "S", "U", "X", "Y", "Z", "L"]) {
    assert.doesNotMatch(stdout, new RegExp(`row ${id}\\b|local ${id}\\b`), id);
  }
});

test("reads only the screens of rows that could have ended", () => {
  const { calls } = fullRun();
  const read = calls.filter((c) => c[1] === "text").map((c) => c[c.indexOf("--target") + 1]);
  assert.deepStrictEqual(read.sort(), ["A", "E", "P", "Q", "S", "S", "T", "U", "K", "V"].sort());
});

test("--dry-run lists the rows and closes nothing", () => {
  const { stdout, closes } = run("--dry-run");
  assert.deepStrictEqual(closes, []);
  assert.match(stdout, /^would close row A \(box\)$/m);
  assert.match(stdout, /^would close row V \(box\)$/m);
  assert.doesNotMatch(stdout, /^Closed/m);
});

test("fails when the app cannot be read, rather than reporting nothing ended", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "close-ended-"));
  try {
    const stub = path.join(dir, "ngtermctl");
    fs.writeFileSync(stub, "#!/bin/sh\necho 'cannot connect to the control socket' >&2\nexit 1\n", { mode: 0o755 });
    const env = { ...process.env, NGTERMCTL: stub };
    delete env.AGT_WINDOW_ID;
    const out = require("node:child_process").spawnSync(SCRIPT, [], { env, encoding: "utf8" });
    assert.notStrictEqual(out.status, 0);
    assert.doesNotMatch(out.stdout, /No ended remote sessions/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("says so when nothing has ended", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "close-ended-"));
  try {
    const stub = path.join(dir, "ngtermctl");
    fs.writeFileSync(stub, STUB, { mode: 0o755 });
    const fixture = { windows: [{ id: "W1", open: true }], trees: { W1: { workspaces: [] } }, texts: {} };
    fs.writeFileSync(path.join(dir, "fixture.json"), JSON.stringify(fixture));
    const env = { ...process.env, NGTERMCTL: stub, FIXTURE: path.join(dir, "fixture.json"), LOG: path.join(dir, "log") };
    delete env.AGT_WINDOW_ID;
    assert.strictEqual(execFileSync(SCRIPT, [], { env, encoding: "utf8" }), "No ended remote sessions\n");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
