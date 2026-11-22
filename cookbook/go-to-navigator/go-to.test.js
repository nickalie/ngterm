#!/usr/bin/env node
"use strict";
// The navigator's rows and keys over a fixed tree, and the script against a stand-in ngtermctl:
// `node --test go-to.test.js` (the script tests need bash, jq and git).
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { Navigator, Fit, Panel, agentOf, workspacesOf, rows, fuzzy, parts, age, pageZoom, columnLimits, stackHeight } =
  require("./assets/go-to.js");

const tree = {
  workspaces: [
    {
      id: "W1", name: "api",
      sessions: [
        { id: "S1", name: "server", cwd: "/src/api", foreground: ["npx", "claude"], status: "active", statusPane: "left" },
        { id: "S2", name: "review", cwd: "/src/api", hasSplit: true, splitCwd: "/src/api/docs",
          foreground: ["codex"], status: "blocked", statusPane: "left", active: true, splitFocused: true },
        { id: "S3", name: "worker", cwd: "/src/api/w", parent: "S2", foreground: ["claude"], status: "completed" },
        { id: "S4", name: "report", parent: "S2", content: "page" },
      ],
    },
    {
      id: "W2", name: "infra",
      sessions: [{ id: "S5", name: "shell", cwd: "/srv", remoteHost: "box" }],
    },
  ],
};
const branches = { "/src/api": "main", "/srv": "ignored-for-remote" };

const lines = list => list.map(r => `${r.tree.join("")}${r.label}${r.pane ? ` [${r.pane.agent} ${r.pane.status}]` : ""}`);
const press = (nav, key, extra = {}) => nav.key({ key, code: "", shift: false, ctrl: false, ...extra });
const letter = (nav, ch) => press(nav, ch, { code: `Key${ch.toUpperCase()}`, shift: ch !== ch.toLowerCase() });
const lit = (text, marks) => parts(text, marks).map(p => (p.mark ? `[${p.text}]` : p.text)).join("");

test("agents are read from the foreground argv, wrappers included", () => {
  assert.equal(agentOf(["npx", "claude"]), "claude");
  assert.equal(agentOf(["/usr/local/bin/codex", "--yolo"]), "codex");
  assert.equal(agentOf(["vim", "notes"]), null);
  assert.equal(agentOf(undefined), null);
});

test("rows nest sessions, panes and children as the sidebar does, without page companions", () => {
  const ws = workspacesOf(tree, branches);
  assert.equal(ws[0].branch, "main");
  assert.equal(ws[1].branch, null, "a remote session has no local branch");
  assert.deepEqual(lines(rows(ws, "", null)), [
    "api",
    "├─server [claude working]",
    "└─review",
    "  ├─codex · review [codex blocked]",
    "  ├─terminal · review [terminal shell]",
    "  └─worker [claude done]",
    "infra",
    "└─shell [terminal shell]",
  ]);
});

test("a split pane without the status reads unknown when an agent runs there", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[1].splitForeground = ["claude"];
  const review = workspacesOf(t, {})[0].panes.filter(p => p.session === "S2");
  assert.deepEqual(review.map(p => p.status), ["blocked", "unknown"]);
});

test("a session's title matches its primary pane only", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[1].title = "fix autostart";
  const list = rows(workspacesOf(t, {}), "autostart", null);
  assert.deepEqual(lines(list), ["api", "└─review", "  └─codex · review [codex blocked]"]);
  assert.equal(list[2].hint, "fix autostart", "the row shows the title it matched by");
});

test("a match hint shows only what the row does not already show", () => {
  const ws = workspacesOf(tree, branches);
  const byCwd = rows(ws, "docs", null).find(r => r.pane);
  assert.ok(!byCwd.hint, "the cwd column already shows the match");
  assert.equal(lit(byCwd.pane.cwd, byCwd.marks.cwd), "/src/api/[docs]");
  assert.ok(rows(ws, "server", null).every(r => !r.hint), "a label match needs no hint");
  assert.ok(rows(ws, "api", null).every(r => !r.hint), "a workspace match keeps its panes without hints");
  assert.ok(rows(ws, "", null).every(r => !r.hint));
});

test("status filters keep their panes and hide empty workspaces", () => {
  const ws = workspacesOf(tree, branches);
  assert.deepEqual(lines(rows(ws, "", "b")), ["api", "└─review", "  └─codex · review [codex blocked]"]);
  assert.deepEqual(lines(rows(ws, "", "d")), ["api", "└─review", "  └─worker [claude done]"]);
  assert.deepEqual(lines(rows(ws, "", "w")), ["api", "└─server [claude working]"]);
});

test("the top-level filter shows one row per top-level session", () => {
  const ws = workspacesOf(tree, branches);
  const list = rows(ws, "", "t");
  assert.deepEqual(lines(list), ["api", "├─server [claude working]", "└─review [codex blocked]", "infra", "└─shell [terminal shell]"]);
  assert.equal(list.reduce((n, r) => n + r.terminals, 0), 4);
});

test("every query word must match one field; a matching workspace keeps all its panes", () => {
  const ws = workspacesOf(tree, branches);
  assert.deepEqual(lines(rows(ws, "docs", null)), ["api", "└─review", "  └─terminal · review [terminal shell]"]);
  assert.deepEqual(lines(rows(ws, "worker claude", null)), []);
  assert.equal(rows(ws, "main", null).filter(r => r.pane).length, 4, "the branch matches the workspace");
  assert.deepEqual(lines(rows(ws, "INFRA", null)), ["infra", "└─shell [terminal shell]"]);
});

test("the window opens on the focused pane and Enter opens it", () => {
  const nav = new Navigator(tree, branches);
  const v = nav.view();
  assert.equal(v.rows[v.selected].target, "p:S2:right");
  assert.equal(v.count, "5 terminals");
  assert.deepEqual(press(nav, "Enter"), { handled: true, open: "p:S2:right" });
});

test("list keys move, jump workspaces and filter", () => {
  const nav = new Navigator(tree, branches);
  letter(nav, "j");
  assert.equal(nav.openTarget(), "p:S3:left");
  letter(nav, "k");
  letter(nav, "k");
  assert.equal(nav.openTarget(), "p:S2:left");
  press(nav, "ArrowRight");
  assert.equal(nav.openTarget(), "p:S5:left");
  press(nav, "ArrowLeft");
  assert.equal(nav.openTarget(), "p:S1:left");
  letter(nav, "G");
  assert.equal(nav.openTarget(), "p:S5:left");
  press(nav, "Home");
  assert.equal(nav.openTarget(), "p:S1:left");

  letter(nav, "w");
  assert.equal(nav.view().filter, "working");
  press(nav, "Backspace");
  assert.equal(nav.view().filter, null);
  assert.equal(nav.view().placeholder, true);
});

test("the top-level filter moves the selection to its top-level session", () => {
  const nav = new Navigator(tree, branches);
  letter(nav, "j");
  assert.equal(nav.openTarget(), "p:S3:left");
  letter(nav, "t");
  assert.equal(nav.openTarget(), "s:S2");
  letter(nav, "a");
  assert.equal(nav.openTarget(), "s:S2", "dropping `t` keeps the selection");
});

test("search takes the keys until Escape, which then closes", () => {
  const nav = new Navigator(tree, branches);
  press(nav, "/", { code: "Slash" });
  for (const ch of "inf") press(nav, ch);
  assert.equal(nav.view().search, "/ inf");
  assert.equal(nav.openTarget(), "p:S5:left");
  letter(nav, "j");
  assert.equal(nav.view().search, "/ infj", "letters type while searching");
  press(nav, "Backspace");
  assert.deepEqual(press(nav, "Escape"), { handled: true });
  assert.equal(nav.view().searching, false);
  assert.deepEqual(press(nav, "Escape"), { handled: true, close: true });
});

test("chords the navigator does not use pass through", () => {
  const nav = new Navigator(tree, branches);
  assert.deepEqual(press(nav, "w", { code: "KeyW", ctrl: true }), { handled: false });
  assert.equal(press(nav, "d", { code: "KeyD", ctrl: true }).handled, true);
});

test("a filter that leaves nothing shows no rows", () => {
  const nav = new Navigator({ workspaces: [] }, {});
  const v = nav.view();
  assert.deepEqual(v.rows, []);
  assert.equal(v.selected, null);
  assert.deepEqual(press(nav, "Enter"), { handled: true });
});

test("list keys follow the letter the layout types, and the position only without a Latin letter", () => {
  const nav = new Navigator(tree, branches);
  // AZERTY: the key typing `w` sits at KeyZ, the one typing `z` at KeyW
  press(nav, "w", { code: "KeyZ" });
  assert.equal(nav.view().filter, "working");
  press(nav, "a", { code: "KeyQ" });
  assert.equal(nav.view().filter, null);
  press(nav, "z", { code: "KeyW" });
  assert.equal(nav.view().filter, null, "z is no filter");
  // Caps Lock: `j` types J without Shift and still moves down; Shift+g is G, the last row
  press(nav, "Home");
  press(nav, "J", { code: "KeyJ" });
  assert.equal(nav.openTarget(), "s:S2");
  press(nav, "G", { code: "KeyG", shift: true });
  assert.equal(nav.openTarget(), "p:S5:left");
  // Cyrillic: the key at KeyJ types о, and still moves down
  press(nav, "Home");
  press(nav, "о", { code: "KeyJ" });
  assert.equal(nav.openTarget(), "s:S2");
});

test("a word fuzzy-matches as letters in order, ignoring case, marked on the shortest stretch that holds them", () => {
  assert.deepEqual(fuzzy("go-to-navigator", "gtn"), [[0, 1], [3, 4], [6, 7]]);
  assert.deepEqual(fuzzy("Server", "SRV"), [[0, 1], [2, 4]]);
  assert.deepEqual(fuzzy("a_b_ab", "ab"), [[4, 6]], "a contiguous run beats scattered letters before it");
  assert.deepEqual(fuzzy("a-a-b", "ab"), [[2, 3], [4, 5]], "the tightest letters before the end");
  assert.deepEqual(fuzzy("a__b_a_b", "ab"), [[5, 6], [7, 8]], "a tighter stretch after the first one to complete");
  assert.deepEqual(fuzzy("a_b_a_b", "ab"), [[0, 1], [2, 3]], "the first of equally short stretches");
  assert.equal(fuzzy("review", "wer"), null, "letters out of order");
  assert.equal(fuzzy("api", "apis"), null);
});

test("search keeps rows whose one field holds every word, and marks the letters that matched", () => {
  const ws = workspacesOf(tree, branches);
  const list = rows(ws, "rvw", null);
  assert.deepEqual(lines(list), ["api", "└─review", "  ├─codex · review [codex blocked]", "  └─terminal · review [terminal shell]"]);
  assert.deepEqual(list.map(r => lit(r.label, r.marks.label)),
                   ["api", "[r]e[v]ie[w]", "codex · [r]e[v]ie[w]", "terminal · [r]e[v]ie[w]"]);

  const docs = rows(ws, "api doc", null);
  assert.deepEqual(lines(docs), ["api", "└─review", "  └─terminal · review [terminal shell]"]);
  assert.equal(lit(docs[2].pane.cwd, docs[2].marks.cwd), "/src/[api]/[doc]s");
  assert.equal(lit(docs[0].label, docs[0].marks.label), "api", "the workspace name holds `api` but not `doc`");

  const byBranch = rows(ws, "man", null);
  assert.equal(lit(byBranch[0].branch, byBranch[0].marks.branch), "[ma]i[n]");
  assert.equal(lit(byBranch[1].pane.branch, byBranch[1].marks.branch), "[ma]i[n]");
  assert.deepEqual(lines(rows(ws, "xyz", null)), []);
});

test("a title match shows dimmed after the label, marked", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[1].title = "fix autostart";
  const row = rows(workspacesOf(t, {}), "fxst", null).find(r => r.pane);
  assert.equal(lit(row.hint, row.marks.hint), "[f]i[x] auto[st]art");
  assert.equal(lit(row.meta, row.metaMarks), "[f]i[x] auto[st]art  /src/api", "the footer puts it before the cwd");
});

test("each pane row shows its branch, its cwd under ~ and how long ago its status was set", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[0].statusChangedAt = 700;
  t.workspaces[0].sessions[1].statusChangedAt = 400;
  const nav = new Navigator(t, { ...branches, "/src/api/docs": "docs-wip" }, { home: "/src", now: () => 1000 });
  const columns = list => list.filter(r => r.pane).map(r => [r.label, r.pane.branch, r.pane.cwd, r.pane.age]);
  assert.deepEqual(columns(nav.view().rows), [
    ["server", "main", "~/api", "5m"],
    ["codex · review", "main", "~/api", "10m"],
    ["terminal · review", "docs-wip", "~/api/docs", ""],
    ["worker", "", "~/api/w", ""],
    ["shell", "", "/srv", ""],
  ]);
  letter(nav, "t");
  assert.deepEqual(columns(nav.view().rows)[1], ["review", "main", "~/api", "10m"], "a top-level row is its session's");

  const remote = new Navigator(tree, branches, { home: "/srv" });
  assert.equal(remote.view().rows.at(-1).pane.cwd, "/srv", "a remote cwd is not under this machine's home");
});

test("a cwd under home reads ~ whether or not home ends in /, and a sibling of home stays whole", () => {
  const t = { workspaces: [{ id: "W", name: "w", sessions: ["/home/u", "/home/u/p", "/home/u2/p", "/home/user"]
    .map((cwd, i) => ({ id: `S${i}`, name: `s${i}`, cwd })) }] };
  const paths = home => workspacesOf(t, {}, home)[0].panes.map(p => p.path);
  for (const home of ["/home/u", "/home/u/", "/home/u//"]) {
    assert.deepEqual(paths(home), ["~", "~/p", "/home/u2/p", "/home/user"], `home ${home}`);
  }
  assert.deepEqual(paths("/"), ["/home/u", "/home/u/p", "/home/u2/p", "/home/user"], "/ is no home to shorten");
  assert.deepEqual(paths(""), ["/home/u", "/home/u/p", "/home/u2/p", "/home/user"]);
});

test("the age of a status reads now, then seconds, minutes, hours and days", () => {
  assert.equal(age(998, 1000), "now");
  assert.equal(age(955, 1000), "45s");
  assert.equal(age(700, 1000), "5m");
  assert.equal(age(1000 - 7200, 1000), "2h");
  assert.equal(age(1000 - 3 * 86400, 1000), "3d");
  assert.equal(age(1005, 1000), "now", "a clock behind the app's");
  assert.equal(age(undefined, 1000), "");
});

test("a long cwd keeps its end, and a match it cuts shows in full as the hint", () => {
  const t = structuredClone(tree);
  t.workspaces[1].sessions[0] = { id: "S5", name: "shell", cwd: "/home/u/projects/very-long-project-name/src/components" };
  const ws = workspacesOf(t, {}, "/home/u");
  const cut = rows(ws, "proj", null).find(r => r.pane);
  assert.equal(cut.pane.cwd, "…/src/components");
  assert.deepEqual(cut.marks.cwd, []);
  assert.equal(lit(cut.hint, cut.marks.hint), "~/[proj]ects/very-long-project-name/src/components");
  const end = rows(ws, "comp", null).find(r => r.pane);
  assert.equal(lit(end.pane.cwd, end.marks.cwd), "…/src/[comp]onents");
  assert.ok(!end.hint);
  const part = rows(ws, "pcomp", null).find(r => r.pane);
  assert.equal(lit(part.pane.cwd, part.marks.cwd), "…/src/[comp]onents");
  assert.equal(lit(part.hint, part.marks.hint), "~/projects/very-long-[p]roject-name/src/[comp]onents",
               "a match the column shows only in part");
});

test("a cwd match the column hides is marked on the footer's full cwd", () => {
  const t = structuredClone(tree);
  t.workspaces[1].sessions[0] = { id: "S5", name: "shell", cwd: "/home/u/projects/very-long-project-name/src/components" };
  const ws = workspacesOf(t, {}, "/home/u");
  const footer = query => {
    const row = rows(ws, query, null).find(r => r.pane);
    return lit(row.meta, row.metaMarks);
  };
  assert.equal(footer("proj"), "/home/u/[proj]ects/very-long-project-name/src/components");
  assert.equal(footer("~proj"), "[/home/u]/[proj]ects/very-long-project-name/src/components", "~ marks the home it stands for");
  assert.equal(footer("comp"), "/home/u/projects/very-long-project-name/src/components", "no hint, no marks");
  const remote = rows(workspacesOf(tree, branches, "/home/u"), "sr", null).find(r => r.target === "p:S5:left");
  assert.equal(lit(remote.meta, remote.metaMarks), "/srv", "a cwd the column shows whole is not marked again");
});

test("a long branch keeps its start, and a match it cuts shows in full as the hint", () => {
  const long = { ...branches, "/src/api/w": "feature/very-long-branch-name-xyz" };
  assert.equal(new Navigator(tree, long).view().widths.branch, 20);
  const ws = workspacesOf(tree, long);
  const row = query => rows(ws, query, null).find(r => r.target === "p:S3:left");
  const start = row("feat");
  assert.equal(lit(start.pane.branch, start.marks.branch), "[feat]ure/very-long-b…");
  assert.ok(!start.hint);
  const end = row("xyz");
  assert.equal(end.pane.branch, "feature/very-long-b…");
  assert.deepEqual(end.marks.branch, []);
  assert.equal(lit(end.hint, end.marks.hint), "feature/very-long-branch-name-[xyz]");
  const part = row("fxyz");
  assert.equal(lit(part.pane.branch, part.marks.branch), "[f]eature/very-long-b…");
  assert.equal(lit(part.hint, part.marks.hint), "[f]eature/very-long-branch-name-[xyz]",
               "a match the column shows only in part");
  assert.equal(lit(end.meta, end.metaMarks), "feature/very-long-branch-name-[xyz]  /src/api/w",
               "the footer shows the branch before the cwd");
  assert.equal(lit(start.meta, start.metaMarks), "/src/api/w");
});

test("a footer line too long for the panel drops the cwd, then keeps its first match in view", () => {
  const footer = (session, query) => {
    const t = structuredClone(tree);
    t.workspaces[1].sessions[0] = { id: "S5", name: "shell", cwd: "/srv", ...session };
    const nav = new Navigator(t, branches, { home: "/home/u" });
    nav.setRoom(88.5, 1.28);
    press(nav, "/", { code: "Slash" });
    for (const ch of query) press(nav, ch);
    const row = nav.view().rows.find(r => r.target === "p:S5:left");
    return lit(row.meta, row.metaMarks);
  };
  const x = n => "x".repeat(n);
  assert.equal(footer({ title: `${x(70)} zzq` }, "zzq"), `${x(70)} [zzq]  /srv`, "the hint and the cwd fit");
  assert.equal(footer({ title: `${x(80)} zzq` }, "zzq"), `${x(80)} [zzq]`, "the hint alone");
  const title = `${"overlay resize ".repeat(6)}zzq`;
  assert.equal(footer({ title }, "zzq"), `…${title.slice(6, -3)}[zzq]`, "a match past the edge keeps the end");
  assert.equal(footer({ title: `${"a".repeat(100)}zzq${"b".repeat(97)}` }, "zzq"), `…[zzq]${"b".repeat(83)}…`,
               "a match in the middle cuts both sides");
  const cwd = `/home/u/${"deep/".repeat(16)}marker/${"tail/".repeat(4)}end`;
  assert.equal(footer({ cwd }, "marker"), `…${cwd.slice(31).replace("marker", "[marker]")}`, "a full cwd too");
});

test("a label match is marked on the footer's workspace / session line, which no narrow label cuts", () => {
  const ws = workspacesOf(tree, branches);
  const detail = (query, target) => {
    const row = rows(ws, query, null).find(r => r.target === target);
    return lit(row.detail, row.detailMarks);
  };
  assert.equal(detail("srv", "p:S1:left"), "api / [s]e[rv]er");
  assert.equal(detail("rvw", "s:S2"), "api / [r]e[v]ie[w]");
  assert.equal(detail("rvw", "p:S2:left"), "api / [r]e[v]ie[w] / codex");
  assert.equal(detail("cdx rvw", "p:S2:left"), "api / [r]e[v]ie[w] / [c]o[d]e[x]", "a split pane's kind too");
  assert.equal(detail("inf", "w:W2"), "[inf]ra");
  assert.equal(detail("docs", "p:S2:right"), "api / review / terminal", "a cwd match leaves it unmarked");
});

const LABEL_ROOM = 16;
const used = (need, gap, out) => {
  const cells = [need.tree, LABEL_ROOM, 1, need.agent, 9, need.age, out.branch && out.branch + 0.5, out.cwd && out.cwd + 0.5]
    .filter(Boolean);
  return cells.reduce((n, w) => n + w, 0) + gap * (cells.length - 1);
};

test("the branch and cwd columns get what the panel leaves after the other cells and a 16-character label", () => {
  const need = { tree: 5, agent: 9, branch: 33, cwd: 47, age: 4 };
  assert.deepEqual(columnLimits(null, 0, need), { branch: 20, cwd: 28 }, "unmeasured: the longest caps");
  assert.deepEqual(columnLimits(140, 1.28, need), { branch: 20, cwd: 28 }, "a wide panel keeps the caps");
  assert.deepEqual(columnLimits(86, 1.28, need), { branch: 16, cwd: 16 }, "a 730 px panel cuts the longer first");
  assert.deepEqual(columnLimits(80, 1.28, { ...need, branch: 4 }), { branch: 4, cwd: 22 }, "a short branch stays whole");
  assert.deepEqual(columnLimits(60, 1.28, need), { branch: 7, cwd: 0 }, "a cwd under 6 characters goes first");
  assert.deepEqual(columnLimits(50, 1.28, need), { branch: 0, cwd: 0 });
  assert.deepEqual(columnLimits(140, 1.28, { ...need, branch: 0 }), { branch: 0, cwd: 28 }, "no branch, no column");
});

test("the cut columns never overflow a row whose label is at its 16 characters", () => {
  const needs = [
    { tree: 3, agent: 9, branch: 33, cwd: 47, age: 4 },
    { tree: 9, agent: 12, branch: 30, cwd: 5, age: 0 },
    { tree: 5, agent: 9, branch: 0, cwd: 60, age: 4 },
    { tree: 7, agent: 9, branch: 25, cwd: 0, age: 0 },
  ];
  for (const need of needs) {
    for (const gap of [0, 1.28, 2.5]) {
      let last = { branch: 0, cwd: 0 };
      const count = out => (out.branch > 0) + (out.cwd > 0);
      for (let room = 20; room <= 200; room += 0.25) {
        const out = columnLimits(room, gap, need);
        const at = `room ${room}, gap ${gap}, need ${JSON.stringify(need)}`;
        if (out.branch || out.cwd) assert.ok(used(need, gap, out) <= room + 1e-9, at);
        assert.ok(out.branch <= Math.min(need.branch, 20) && out.cwd <= Math.min(need.cwd, 28), at);
        for (const k of ["branch", "cwd"]) {
          assert.ok(out[k] === 0 || out[k] >= Math.min(need[k], 6), `${k} too narrow to read, ${at}`);
        }
        assert.ok(count(out) >= count(last), `a wider panel never drops a column, ${at}`);
        last = out;
      }
      assert.deepEqual(last, { branch: Math.min(need.branch, 20), cwd: Math.min(need.cwd, 28) });
    }
  }
});

test("a narrow panel cuts the columns to its width; a match the cut hides shows as the hint", () => {
  const t = structuredClone(tree);
  t.workspaces[1].sessions[0] = { id: "S5", name: "shell", cwd: "/home/u/projects/very-long-project-name/src/components" };
  const long = { ...branches, "/src/api/w": "feature/very-long-branch-name-xyz" };
  const nav = new Navigator(t, long, { home: "/home/u", now: () => 1000 });
  assert.ok(!rows(workspacesOf(t, long, "/home/u"), "long", null).find(r => r.target === "p:S3:left").hint,
            "a 20-character branch shows `long`");
  nav.setRoom(70, 1.28);
  const v = nav.view();
  assert.deepEqual(v.widths, { branch: 11, cwd: 10, age: 0 });
  for (const r of v.all.filter(x => x.pane)) {
    assert.ok([...r.pane.branch].length <= v.widths.branch && [...r.pane.cwd].length <= v.widths.cwd, r.label);
  }
  const need = { tree: 5, agent: 9, branch: v.widths.branch, cwd: v.widths.cwd, age: 0 };
  assert.ok(used(need, 1.28, v.widths) <= 70);
  press(nav, "/", { code: "Slash" });
  for (const ch of "long") press(nav, ch);
  const worker = nav.view().rows.find(r => r.target === "p:S3:left");
  assert.equal(lit(worker.pane.branch, worker.marks.branch), "feature/ve…");
  assert.equal(lit(worker.hint, worker.marks.hint), "feature/very-[long]-branch-name-xyz");
  assert.deepEqual(nav.view().widths, v.widths, "the search keeps the columns where they were");
  nav.setRoom(140, 1.28);
  assert.deepEqual(nav.view().widths, { branch: 20, cwd: 16, age: 0 }, "a resize to a wide panel lifts the cut");
});

test("an active filter shows as its own chip and lights its key in the hints", () => {
  const nav = new Navigator(tree, branches);
  assert.equal(nav.view().filter, null);
  assert.ok(!lit(nav.view().hints, nav.view().hintMarks).includes("["));
  letter(nav, "b");
  let v = nav.view();
  assert.equal(v.filter, "blocked");
  assert.equal(v.search, "/ search agents and terminals", "the search text stays apart from the filter");
  assert.ok(lit(v.hints, v.hintMarks).includes(" a/[b]/w/i/d/t filter "));
  letter(nav, "t");
  v = nav.view();
  assert.equal(v.filter, "top-level");
  assert.ok(lit(v.hints, v.hintMarks).includes(" a/b/w/i/d/[t] filter "));
  press(nav, "/", { code: "Slash" });
  assert.equal(nav.view().filter, null, "a search drops the filter");
});

test("a filter or a search leaves the full list and the column widths the panel is fitted to as they were", () => {
  const nav = new Navigator(tree, branches, { now: () => 1000 });
  const before = nav.view();
  letter(nav, "b");
  const filtered = nav.view();
  assert.ok(filtered.rows.length < before.rows.length);
  assert.deepEqual(filtered.all, before.rows);
  assert.deepEqual(filtered.widths, before.widths);
  press(nav, "/", { code: "Slash" });
  for (const ch of "docs") press(nav, ch);
  const searched = nav.view();
  assert.ok(searched.rows.length < before.rows.length);
  assert.deepEqual(searched.all, before.rows);
  assert.deepEqual(searched.widths, before.widths);
});

test("the page asks for a new height only when the one it needs changes, in points at the page zoom", () => {
  const fit = new Fit();
  assert.equal(fit.next(300.2, 1), 301);
  assert.equal(fit.next(300.2, 1), null, "unchanged");
  assert.equal(fit.next(300.2, 1.25), 376);
  assert.equal(fit.next(100, 1.1), 110);
  assert.equal(fit.next(320, undefined), 320, "no zoom reads as actual size");
  assert.equal(fit.next(0, 1), null);
  assert.equal(fit.next(Number.NaN, 1), null);
});

test("the panel height is its blocks, the gaps between them and the padding", () => {
  assert.equal(stackHeight([30, 200, 40, 17], 8, 20), 331);
  assert.equal(stackHeight([50], 8, 20), 70);
});

function bridge(trees) {
  const calls = [];
  const request = (cmd, opts) => {
    calls.push(opts === undefined ? [cmd] : [cmd, opts]);
    return Promise.resolve(cmd === "tree" ? { tree: trees.shift() } : {});
  };
  return { calls, request, resizes: () => calls.filter(c => c[0] === "session.overlay.resize").map(c => c[1]) };
}

test("the panel fits the full list, filtered or not, once per changed tree, in points", async () => {
  const grown = structuredClone(tree);
  grown.workspaces[1].sessions.push({ id: "S6", name: "logs", cwd: "/srv" });
  const restatus = structuredClone(grown);
  restatus.workspaces[0].sessions[0].status = "blocked";
  const zoomed = structuredClone(restatus);
  zoomed.workspaces[0].sessions[1].htmlOverlays = [{ javascript: true, zoom: 2 }];
  const fake = bridge([tree, tree, grown, restatus, zoomed]);
  const measured = [];
  const panel = new Panel(fake.request, list => {
    measured.push(list.length);
    return list.length * 10 + 0.4;
  }, "S2");
  const nav = new Navigator(tree, branches);
  letter(nav, "b");
  const apply = t => {
    nav.setTree(t);
    return nav.view();
  };

  assert.equal(await panel.poll(apply), 81);
  assert.deepEqual(fake.calls, [["tree"], ["session.overlay.resize", { args: { height: 81 } }]]);
  assert.deepEqual(measured, [8], "the full list's 8 rows, not the filter's 3");

  assert.equal(await panel.poll(apply), null);
  assert.deepEqual(measured, [8], "the same tree is not measured again");
  assert.equal(await panel.poll(apply), 91, "a new session grows the panel");
  assert.equal(await panel.poll(apply), null, "a status change measures the same height");
  assert.deepEqual(measured, [8, 9, 9]);
  assert.equal(await panel.poll(apply), 181, "points at the page's zoom");
  assert.deepEqual(fake.resizes(), [{ args: { height: 81 } }, { args: { height: 91 } }, { args: { height: 181 } }]);
});

test("without a bridge the panel neither polls, measures nor resizes", async () => {
  let measured = 0;
  for (const none of [undefined, null, {}]) {
    const panel = new Panel(none, () => ++measured, "S2");
    assert.equal(await panel.poll(t => new Navigator(t, branches).view()), null);
    assert.equal(panel.place(tree, new Navigator(tree, branches).view()), null);
  }
  assert.equal(measured, 0);
});

test("a bridge answer that is no tree changes nothing", async () => {
  const fake = bridge([null, { workspaces: [] }]);
  let applied = 0;
  const panel = new Panel(fake.request, list => list.length * 10 + 30, "");
  assert.equal(await panel.poll(() => ++applied), null);
  assert.equal(applied, 0);
  fake.calls.length = 0;
  const bare = bridge([]);
  const direct = new Panel((cmd, opts) => (cmd === "tree" ? Promise.resolve(tree) : bare.request(cmd, opts)),
                           list => list.length * 10, "");
  assert.equal(await direct.poll(t => new Navigator(t, branches).view()), 80, "a bare tree is a tree too");
  assert.deepEqual(bare.resizes(), [{ args: { height: 80 } }]);
});

test("the zoom is read from the page's own entry in the tree", () => {
  const t = structuredClone(tree);
  t.workspaces[0].sessions[0].htmlOverlays = [{ pane: "right", javascript: true, zoom: 3 }, { javascript: true, zoom: 1.5 }];
  t.workspaces[0].sessions[1].htmlOverlays = [{ javascript: true, zoom: 2 }];
  assert.equal(pageZoom(t, "s1"), 1.5, "the session-wide page of the session it opened on");
  assert.equal(pageZoom(t, ""), 2, "without a session, the selected one's");
  assert.equal(pageZoom(t, "S5"), 1, "no page there");
  assert.equal(pageZoom(tree, "S1"), 1);
});

// The script with a stand-in ngtermctl that serves `tree` and answers `overlay open` with OPEN_STATUS/OPEN_OUT,
// logging every call.
function runScript(args, { treeJson, openStatus = 0, openOut = "", settings, env: extra = {} } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "go-to-test-"));
  const home = path.join(dir, "home");
  const state = path.join(dir, "state");
  fs.mkdirSync(home);
  fs.mkdirSync(state);
  if (settings !== undefined) fs.writeFileSync(path.join(state, "settings.json"), settings);
  const fake = path.join(dir, "ngtermctl");
  fs.writeFileSync(fake, `#!/usr/bin/env bash
echo "$*" >>"${dir}/calls"
case "$1 $2 $3" in
  "tree --json "*) cat "${dir}/tree.json" ;;
  "session overlay open") cp "$5" "${dir}/page.html"; printf '%s' "$OPEN_OUT"; exit "$OPEN_STATUS" ;;
esac
`, { mode: 0o755 });
  fs.writeFileSync(path.join(dir, "tree.json"), JSON.stringify({ ok: true, result: { tree: treeJson || tree } }));
  const env = Object.fromEntries(Object.entries(process.env)
    .filter(([k]) => !/^(NGTERM|AGT)_|^XDG_DATA_HOME$/.test(k)));
  Object.assign(env, { NGTERMCTL: fake, OPEN_STATUS: String(openStatus), OPEN_OUT: openOut, TMPDIR: dir, HOME: home,
                       NGTERM_STATE_DIR: state }, extra);
  const run = spawnSync("bash", [path.join(__dirname, "go-to-navigator.sh"), ...args], { env, encoding: "utf8" });
  const read = f => (fs.existsSync(path.join(dir, f)) ? fs.readFileSync(path.join(dir, f), "utf8") : "");
  const calls = read("calls").trim().split("\n");
  const open = calls.find(c => c.startsWith("session overlay open")) || "";
  const height = Number((/ --height (\S+)/.exec(open) || [])[1]);
  const result = { status: run.status, stdout: run.stdout, stderr: run.stderr, calls, open, height,
                   page: read("page.html") };
  fs.rmSync(dir, { recursive: true, force: true });
  return result;
}

const pageCss = (() => {
  const html = fs.readFileSync(path.join(__dirname, "assets", "go-to.html"), "utf8");
  const rules = {};
  for (const [, selectors, body] of /<style>([\s\S]*?)<\/style>/.exec(html)[1].matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const decls = body.split(";").filter(d => d.includes(":"))
      .map(d => [d.slice(0, d.indexOf(":")).trim(), d.slice(d.indexOf(":") + 1).trim()]);
    for (const s of selectors.split(",")) rules[s.trim()] = { ...rules[s.trim()], ...Object.fromEntries(decls) };
  }
  return rules;
})();

const pagePx = (() => {
  const px = value => {
    assert.match(value, /^\d+(px)?$/, "a height in go-to.html is a whole number of pixels");
    return parseInt(value, 10);
  };
  const vertical = padding => {
    const sides = padding.split(/\s+/);
    return px(sides[0]) + px(sides[2] ?? sides[0]);
  };
  const line = px(/\/(\S+)/.exec(pageCss.body.font)[1]);
  return {
    top: line + vertical(pageCss["#top"].padding),
    info: 2 * line + vertical(pageCss["#info"].padding),
    hints: px(pageCss["#hints"]["line-height"]) + vertical(pageCss["#hints"].padding),
    gaps: 3 * px(pageCss["#app"].gap),
    padding: vertical(pageCss["#app"].padding),
    row: line + vertical(pageCss[".row"].padding),
    workspace_margin: px(pageCss[".row.ws"]["margin-top"]),
    empty: line + vertical(pageCss[".empty"].padding),
  };
})();

const opensAt = (t, zoom = 1) => {
  const p = pagePx;
  const workspaces = t.workspaces.length;
  const list = workspaces ? p.row * new Navigator(t, {}).view().all.length + p.workspace_margin * (workspaces - 1)
                          : p.empty;
  return new Fit().next(list + p.top + p.info + p.hints + p.gaps + p.padding, zoom);
};

test("outside text cannot open a comment or a tag in the page's data", () => {
  const hostile = structuredClone(tree);
  hostile.workspaces[0].sessions[0].title = "<!--<script>";
  hostile.workspaces[0].name = "</script><b>x";
  const run = runScript([], { treeJson: hostile, openStatus: 2 });
  assert.equal(run.status, 0, run.stderr);
  const data = run.page.split("\n").find(l => l.startsWith("const GOTO = "));
  assert.ok(data && !data.includes("<"), "the data line holds no `<`");
  assert.equal(run.page.match(/<script>/g).length, 3);
  const GOTO = new Function(`${data}; return GOTO;`)();
  assert.equal(GOTO.tree.workspaces[0].sessions[0].title, "<!--<script>");
});

test("the page carries the home directory, its session and the branch of each local pane's directory", () => {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "go-to-repo-"));
  spawnSync("git", ["init", "-q", repo]);
  spawnSync("git", ["-C", repo, "symbolic-ref", "HEAD", "refs/heads/topic"]);
  const t = structuredClone(tree);
  t.workspaces[0].sessions[1].splitCwd = repo;
  const run = runScript([], { treeJson: t, openStatus: 2, env: { HOME: "/home/u", AGT_SESSION_ID: "S2" } });
  fs.rmSync(repo, { recursive: true, force: true });
  assert.equal(run.status, 0, run.stderr);
  assert.ok(run.calls.some(c => c.startsWith("session overlay open") && c.includes("--target S2")));
  const data = run.page.split("\n").find(l => l.startsWith("const GOTO = "));
  const GOTO = new Function(`${data}; return GOTO;`)();
  assert.equal(GOTO.home, "/home/u");
  assert.equal(GOTO.session, "S2");
  assert.equal(GOTO.branches[repo], "topic");
  const nav = new Navigator(GOTO.tree, GOTO.branches, { home: GOTO.home });
  assert.equal(nav.view().rows.find(r => r.target === "p:S2:right").pane.branch, "topic");
});

test("a dismissed page does nothing; any other overlay failure keeps its status", () => {
  const dismissed = runScript([], { openStatus: 2 });
  assert.equal(dismissed.status, 0);
  assert.ok(!dismissed.calls.some(c => c.includes(" select ")));
  const refused = runScript([], { openStatus: 1 });
  assert.equal(refused.status, 1);
});

test("a chosen pane is selected and focused; a session only selected", () => {
  const pane = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "p:S2:right" }) });
  assert.equal(pane.status, 0, pane.stderr);
  assert.ok(pane.calls.includes("session select --target S2"));
  assert.ok(pane.calls.includes("session focus right --target S2"));
  const session = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "s:S1" }) });
  assert.ok(session.calls.includes("session select --target S1"));
  assert.ok(!session.calls.some(c => c.startsWith("session focus")));
  const ws = runScript([], { openOut: JSON.stringify({ outcome: "submitted", value: "w:W2" }) });
  assert.ok(ws.calls.includes("workspace select --target W2"));
});

test("the panel opens at the height the page fits it to: the full list's rows, in points", () => {
  const t = structuredClone(tree);
  t.workspaces.push({ id: "W3", name: "empty", sessions: [] });
  assert.equal(new Navigator(t, {}).view().all.length, 9, "split panes, a child and an empty workspace; no page");
  const run = runScript([], { treeJson: t, openStatus: 2 });
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.height, opensAt(t));
  assert.match(run.open, / --size-percent 80 --height \d+ /, "the width stays 80% of the pane");
});

test("the panel opens at the page zoom in settings.json, clamped as the app clamps it, else at actual size", () => {
  const cases = [
    ['{"htmlOverlayZoom": 1.25}', 1.25],
    ['{"htmlOverlayZoom": 7}', 3],
    ['{"htmlOverlayZoom": 0.1}', 0.5],
    ['{"htmlOverlayZoom": "large"}', 1],
    ['{"htmlOverlayZoom": null}', 1],
    ['{"htmlOverlayZoom": ', 1],
    [undefined, 1],
  ];
  for (const [settings, zoom] of cases) {
    const run = runScript([], { openStatus: 2, settings });
    assert.equal(run.status, 0, run.stderr);
    assert.equal(run.height, opensAt(tree, zoom), String(settings));
  }
});

test("with no workspace the panel opens at the empty list's one line", () => {
  const run = runScript([], { treeJson: { workspaces: [] }, openStatus: 2 });
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.height, opensAt({ workspaces: [] }));
});

test("the script adds up the heights go-to.html lays out, and rounds as the page does", () => {
  const script = fs.readFileSync(path.join(__dirname, "go-to-navigator.sh"), "utf8");
  const page = fs.readFileSync(path.join(__dirname, "assets", "go-to.js"), "utf8");
  const scriptPx = Object.fromEntries([.../\{([^}]*)\} as \$px/.exec(script)[1].matchAll(/(\w+): (\d+)/g)]
    .map(([, name, value]) => [name, Number(value)]));
  assert.deepEqual(scriptPx, pagePx);
  assert.equal(/\* \$zoom - (\S+) \| ceil/.exec(script)[1], /: 1\) - (\S+)\);/.exec(page)[1]);
});

test("WebKit's float noise in a measure keeps its point, a hundredth of a pixel more takes the next", () => {
  assert.equal(new Fit().next(315.0000067, 1), 315);
  assert.equal(new Fit().next(180.0000038, 1.75), 315);
  assert.equal(new Fit().next(315.01, 1), 316);
});

test("settings.json beside the app's socket wins over the state directory the environment names", () => {
  const app = fs.mkdtempSync(path.join(os.tmpdir(), "go-to-app-"));
  fs.writeFileSync(path.join(app, "settings.json"), JSON.stringify({ htmlOverlayZoom: 2 }));
  const at = env => runScript([], { openStatus: 2, settings: '{"htmlOverlayZoom": 1.5}', env }).height;
  assert.equal(at({ NGTERM_SOCKET: path.join(app, "ngterm.sock") }), opensAt(tree, 2), "a shell's socket");
  assert.equal(at({ AGT_SOCKET: path.join(app, "ngterm.sock") }), opensAt(tree, 2), "a chord's socket");
  assert.equal(at({ AGT_SOCKET: path.join(app, "other", "ngterm.sock") }), opensAt(tree, 1.5), "no settings.json there");
  assert.equal(at({ AGT_SOCKET: path.join(app, "control.sock") }), opensAt(tree, 1.5), "a socket not in a state dir");
  fs.rmSync(app, { recursive: true, force: true });
});

test("settings.json is read from the state directory the app uses", () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "go-to-home-"));
  for (const [dir, zoom] of [[".local/share", 2], ["data", 1.5]]) {
    fs.mkdirSync(path.join(home, dir, "ngterm"), { recursive: true });
    fs.writeFileSync(path.join(home, dir, "ngterm", "settings.json"), JSON.stringify({ htmlOverlayZoom: zoom }));
  }
  const at = env => runScript([], { openStatus: 2, settings: '{"htmlOverlayZoom": 3}', env: { HOME: home, ...env } });
  const unset = { NGTERM_STATE_DIR: "" };
  assert.equal(at({}).height, opensAt(tree, 3), "NGTERM_STATE_DIR first");
  assert.equal(at(unset).height, opensAt(tree, 2), "an empty one is unset: ~/.local/share/ngterm");
  assert.equal(at({ ...unset, XDG_DATA_HOME: path.join(home, "data") }).height, opensAt(tree, 1.5));
  assert.equal(at({ ...unset, XDG_DATA_HOME: "data" }).height, opensAt(tree, 2), "a relative XDG_DATA_HOME is ignored");
  fs.rmSync(home, { recursive: true, force: true });
});
