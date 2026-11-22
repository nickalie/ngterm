"use strict";
// The Go to window's model: a window's workspaces as a tree of their sessions' panes, searched by text and
// filtered by agent status. Rows and keys follow ngterm PR #28's navigator, itself after herdr's.
const GoTo = (() => {
  const PLACEHOLDER = "/ search agents and terminals";
  const NO_MATCH = "No matching agents or terminals";
  const FILTER_KEYS = "a/b/w/i/d/t";
  const HINTS = `↑↓/j/k rows · ←→ workspace · / search · ${FILTER_KEYS} filter · enter open · esc close`;
  const SEARCH_HINTS = "search type · move ↑↓/ctrl+n/p · open enter · back esc";
  const PAGE = 8;
  const FILTERS = { b: "blocked", w: "working", i: "idle", d: "done", t: "top-level" };
  const CWD_MAX = 28;
  const BRANCH_MAX = 20;
  const AGE_WIDTH = 4;
  const AGENT_WIDTH = 9;
  const STATUS_WIDTH = 9;
  const LABEL_ROOM = 16;
  const CUT_MIN = 6;
  const SLACK = 0.5;
  const CAPS = { branch: BRANCH_MAX, cwd: CWD_MAX, line: Infinity };
  const WHOLE = { branch: Infinity, cwd: Infinity };
  const AGENTS = ["claude", "codex", "opencode", "gemini", "aider", "amp", "pi", "omp", "crush", "goose",
                  "cursor-agent", "copilot", "qwen", "droid", "kiro-cli", "kimi", "auggie"];
  const STATUS = { active: "working", blocked: "blocked", completed: "done" };

  const target = {
    workspace: id => `w:${id}`,
    session: id => `s:${id}`,
    pane: (id, side) => `p:${id}:${side}`,
  };

  function sessionOf(t) {
    const m = /^[sp]:([^:]+)/.exec(t || "");
    return m ? m[1] : null;
  }

  // The agent a pane runs, read from its foreground argv: a wrapper such as `npx claude` still names it.
  function agentOf(argv) {
    if (!Array.isArray(argv)) return null;
    for (const arg of argv) {
      const base = String(arg).split("/").pop();
      if (AGENTS.includes(base)) return base;
    }
    return null;
  }

  function tildePath(path, home) {
    const root = home && home !== "/" ? home.replace(/\/+$/, "") : "";
    if (!root || !path) return path;
    if (path === root) return "~";
    return path.startsWith(`${root}/`) ? `~${path.slice(root.length)}` : path;
  }

  function age(changedAt, now) {
    if (typeof changedAt !== "number" || !Number.isFinite(changedAt)) return "";
    const s = Math.max(0, Math.floor(now - changedAt));
    if (s < 5) return "now";
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }

  function spans(indices) {
    const out = [];
    for (const i of indices) {
      const last = out[out.length - 1];
      if (last && last[1] === i) last[1] = i + 1;
      else out.push([i, i + 1]);
    }
    return out;
  }

  const letters = text => [...String(text)].map(c => c.toLowerCase());

  function fuzzy(text, word) {
    const s = letters(text);
    const w = letters(word);
    if (!w.length) return [];
    let best = null;
    for (let from = 0; ;) {
      let end = -1;
      for (let i = from, k = 0; i < s.length; i++) {
        if (s[i] === w[k] && ++k === w.length) {
          end = i;
          break;
        }
      }
      if (end < 0) break;
      const at = [];
      for (let i = end, k = w.length - 1; k >= 0; i--) {
        if (s[i] === w[k]) {
          at.unshift(i);
          k--;
        }
      }
      if (!best || end - at[0] < best[best.length - 1] - best[0]) best = at;
      from = at[0] + 1;
    }
    return best && spans(best);
  }

  function matcher(query) {
    const words = query.trim().split(/\s+/).filter(Boolean);
    const hits = text => {
      if (!words.length) return [];
      if (text === null || text === undefined || text === "") return null;
      const at = new Set();
      for (const w of words) {
        const found = fuzzy(text, w);
        if (!found) return null;
        for (const [s, e] of found) for (let i = s; i < e; i++) at.add(i);
      }
      return spans([...at].sort((a, b) => a - b));
    };
    return { words, hits, matches: text => hits(text) !== null };
  }

  function parts(text, marks) {
    const cps = [...String(text)];
    const out = [];
    let at = 0;
    for (const [s, e] of marks || []) {
      if (s > at) out.push({ text: cps.slice(at, s).join(""), mark: false });
      if (e > Math.max(s, at)) out.push({ text: cps.slice(Math.max(s, at), e).join(""), mark: true });
      at = Math.max(at, e);
    }
    if (at < cps.length) out.push({ text: cps.slice(at).join(""), mark: false });
    return out.filter(p => p.text);
  }

  const size = marks => marks.reduce((n, [s, e]) => n + e - s, 0);

  function cutHead(text, marks, cut) {
    return {
      text: `…${[...text].slice(cut).join("")}`,
      marks: marks.filter(([, e]) => e > cut).map(([s, e]) => [Math.max(s, cut) - cut + 1, e - cut + 1]),
    };
  }

  function clip(text, marks, max) {
    const cps = [...text];
    if (cps.length <= max) return { text, marks };
    if (max < 1) return { text: "", marks: [] };
    const least = cps.length - max + 1;
    const slash = cps.indexOf("/", least);
    return cutHead(text, marks, slash >= 0 && cps.length - slash >= max / 2 ? slash : least);
  }

  function clipTail(text, marks, max) {
    const cps = [...text];
    if (cps.length <= max) return { text, marks };
    if (max < 1) return { text: "", marks: [] };
    const keep = max - 1;
    return {
      text: `${cps.slice(0, keep).join("")}…`,
      marks: marks.filter(([s]) => s < keep).map(([s, e]) => [s, Math.min(e, keep)]),
    };
  }

  function around(text, marks, max) {
    const length = [...text].length;
    if (length <= max || !marks.length || marks[0][1] < max) return clipTail(text, marks, max);
    const head = cutHead(text, marks, Math.min(marks[0][0], length - max + 1));
    return clipTail(head.text, head.marks, max);
  }

  const whole = (shown, found) => shown.length > 0 && size(shown) === size(found);

  function workspacesOf(tree, branches, home) {
    const known = branches || {};
    return ((tree && tree.workspaces) || []).map(ws => {
      const sessions = (ws.sessions || []).filter(s => s.content !== "page");
      const panes = [];
      for (const s of sessions) {
        const split = !!s.hasSplit;
        const holder = split && s.statusPane === "right" ? "right" : "left";
        const local = !s.remoteHost;
        for (const side of split ? ["left", "right"] : ["left"]) {
          const agent = agentOf(side === "left" ? s.foreground : s.splitForeground);
          const cwd = (side === "right" ? s.splitCwd || s.cwd : s.cwd) || "";
          let status;
          if (side === holder) status = STATUS[s.status] || (agent ? "idle" : "shell");
          else status = agent ? "unknown" : "shell";
          panes.push({
            session: s.id, parent: s.parent || null, pane: side, sessionName: s.name || "", split,
            // the tree's title, like its cwd and foreground, is the primary pane's
            title: side === "left" ? s.title || null : null,
            cwd, path: local ? tildePath(cwd, home) : cwd,
            branch: local && cwd && Object.hasOwn(known, cwd) ? known[cwd] : null,
            changedAt: s.statusChangedAt ?? null, ownsStatus: side === holder,
            agent, status,
            indicator: side === holder && !!STATUS[s.status],
            current: !!s.active && (!split || (side === "right") === !!s.splitFocused),
          });
        }
      }
      const first = sessions[0];
      const branch = first && !first.remoteHost ? (branches || {})[first.cwd] || null : null;
      return { id: ws.id, name: ws.name || "", branch, panes };
    });
  }

  const kind = p => p.agent || "terminal";
  const paneLabel = p => (p.split ? `${kind(p)} · ${p.sessionName}` : p.sessionName);

  // Sessions nested under their parents, in sidebar order.
  function nodes(ws) {
    const made = new Map();
    for (const p of ws.panes) {
      if (!made.has(p.session)) made.set(p.session, { main: p, panes: [], children: [] });
      made.get(p.session).panes.push(p);
    }
    const roots = [];
    for (const node of made.values()) {
      const parent = node.main.parent && made.get(node.main.parent);
      (parent ? parent.children : roots).push(node);
    }
    return roots;
  }

  function kept(node, keeps) {
    const panes = node.panes.filter(keeps);
    const children = node.children.map(c => kept(c, keeps)).filter(Boolean);
    return panes.length || children.length ? { main: node.main, panes, children } : null;
  }
  const holds = (node, session) => node.main.session === session || node.children.some(c => holds(c, session));
  const nodeCurrent = node => node.panes.some(p => p.current) || node.children.some(nodeCurrent);
  const branchLine = (rails, last) => rails.concat([last ? "└─" : "├─"]);
  const columns = p => ({ agent: kind(p), status: p.status, indicator: p.indicator, current: p.current,
                          branch: p.branch || "", path: p.path, changedAt: p.ownsStatus ? p.changedAt : null });

  function workspaceRow(ws) {
    return { target: target.workspace(ws.id), workspace: true, tree: [], label: ws.name, branch: ws.branch,
             detail: ws.name, meta: ws.branch || "", pane: null, terminals: 0 };
  }
  function sessionRow(ws, main, tree) {
    return { target: target.session(main.session), workspace: false, tree, label: main.sessionName,
             detail: `${ws.name} / ${main.sessionName}`, meta: main.cwd, pane: null, terminals: 0 };
  }
  function paneRow(ws, p, tree) {
    const session = `${ws.name} / ${p.sessionName}`;
    return { target: target.pane(p.session, p.pane), workspace: false, tree, label: paneLabel(p),
             detail: p.split ? `${session} / ${kind(p)}` : session, meta: p.cwd, pane: columns(p), terminals: 1,
             title: p.title, kind: p.split ? kind(p) : "" };
  }

  function sessionRows(rows, ws, node, rails, last) {
    const splitPanes = node.main.split ? node.panes : [];
    if (node.panes.length && !node.main.split) rows.push(paneRow(ws, node.panes[0], branchLine(rails, last)));
    else rows.push(sessionRow(ws, node.main, branchLine(rails, last)));
    const inner = rails.concat([last ? "  " : "│ "]);
    const under = splitPanes.length + node.children.length;
    splitPanes.forEach((p, i) => rows.push(paneRow(ws, p, branchLine(inner, i + 1 === under))));
    node.children.forEach((c, i) => sessionRows(rows, ws, c, inner, splitPanes.length + i + 1 === under));
  }

  function topLevelRow(ws, node, tree) {
    let row, holder;
    if (node.main.split) {
      holder = node.panes.find(p => p.indicator) || node.main;
      row = { ...sessionRow(ws, node.main, tree), terminals: node.panes.length };
    } else {
      holder = node.main;
      row = paneRow(ws, node.main, tree);
    }
    const { branch, path, changedAt } = node.main;
    return { ...row, pane: { ...columns(holder), branch: branch || "", path, changedAt, current: nodeCurrent(node) } };
  }

  function paneMatches(p, matches) {
    return matches(p.sessionName) || matches(paneLabel(p)) || matches(p.path) || (!!p.branch && matches(p.branch)) ||
      (!!p.agent && matches(p.agent)) || (!!p.title && matches(p.title));
  }

  function footer(r, path, max) {
    let line;
    if (r.hint === path) {
      const shift = [...r.meta].length - [...path].length;
      line = around(r.meta, r.marks.hint.map(([s, e]) => [s ? s + shift : 0, e + shift]), max);
    } else {
      const both = [r.hint, r.meta].filter(Boolean).join("  ");
      line = around([...both].length <= max ? both : r.hint, r.marks.hint, max);
    }
    r.meta = line.text;
    r.metaMarks = line.marks;
  }

  function detailMarks(r, marks) {
    const k = r.kind ? [...r.kind].length : 0;
    const from = k && k + 3;
    const end = [...r.detail].length;
    const shift = end - [...r.label].length - from;
    return marks.filter(([s]) => s >= from).map(([s, e]) => [s + shift, e + shift])
      .concat(marks.filter(([, e]) => e <= k).map(([s, e]) => [s + end - k, e + end - k]));
  }

  function decorate(r, hits, hinting, now, limits) {
    const marks = { label: hits(r.label) || [], branch: [], cwd: [], agent: [], hint: [] };
    if (r.workspace && r.branch) marks.branch = hits(r.branch) || [];
    r.marks = marks;
    r.metaMarks = [];
    r.detailMarks = detailMarks(r, marks.label);
    if (!r.pane) return;
    const p = r.pane;
    const branch = p.branch;
    const onBranch = hits(branch) || [];
    const shortBranch = clipTail(branch, onBranch, limits.branch);
    p.branch = shortBranch.text;
    marks.branch = shortBranch.marks;
    if (AGENTS.includes(p.agent)) marks.agent = hits(p.agent) || [];
    const found = hits(p.path) || [];
    const cwd = clip(p.path, found, limits.cwd);
    p.cwd = cwd.text;
    marks.cwd = cwd.marks;
    p.age = age(p.changedAt, now);
    const shown = marks.label.length || marks.agent.length || whole(shortBranch.marks, onBranch) ||
      whole(cwd.marks, found);
    if (!hinting || shown) return;
    r.hint = [r.title, branch, p.path].find(v => v && hits(v)) || null;
    if (!r.hint) return;
    marks.hint = hits(r.hint);
    footer(r, p.path, limits.line);
  }

  // Every query word must be in one field; a status filter keeps its panes; `t` keeps top-level sessions.
  function rows(workspaces, query, filter, now = Date.now() / 1000, limits = CAPS) {
    const { words, hits, matches } = matcher(query);
    const status = filter && filter !== "t" ? FILTERS[filter] : null;
    const narrowing = !!status || words.length > 0;
    const out = [];
    for (const ws of workspaces) {
      const wsMatches = matches(ws.name) || (!!ws.branch && matches(ws.branch));
      const keeps = p => (!status || p.status === status) && (wsMatches || paneMatches(p, matches));
      const sessions = nodes(ws).map(n => kept(n, keeps)).filter(Boolean);
      if (!narrowing || sessions.length || (!status && wsMatches)) {
        const first = out.length;
        out.push(workspaceRow(ws));
        sessions.forEach((n, i) => {
          const last = i === sessions.length - 1;
          if (filter === "t") out.push(topLevelRow(ws, n, branchLine([], last)));
          else sessionRows(out, ws, n, [], last);
        });
        for (const r of out.slice(first)) decorate(r, hits, words.length > 0 && !wsMatches, now, limits);
      }
    }
    return out;
  }

  // The selected row: its target's, else its session's, else its session's main pane; with none, the first pane row.
  function selectedIndex(list, selected) {
    if (!list.length) return null;
    const pos = t => {
      const i = list.findIndex(r => r.target === t);
      return i < 0 ? null : i;
    };
    if (selected) {
      const session = sessionOf(selected);
      if (session) return pos(selected) ?? pos(target.session(session)) ?? pos(target.pane(session, "left")) ?? 0;
      return pos(selected) ?? 0;
    }
    const i = list.findIndex(r => r.pane);
    return i < 0 ? 0 : i;
  }

  function topLevelTarget(workspaces, t) {
    const session = sessionOf(t);
    if (!session) return t;
    for (const ws of workspaces) for (const n of nodes(ws)) if (holds(n, session)) return target.session(n.main.session);
    return t;
  }

  const longest = values => values.reduce((n, v) => Math.max(n, [...(v || "")].length), 0);

  function widths(list) {
    const panes = list.filter(r => r.pane).map(r => r.pane);
    return {
      branch: longest(panes.map(p => p.branch)),
      cwd: longest(panes.map(p => p.cwd)),
      age: panes.some(p => p.age) ? AGE_WIDTH : 0,
    };
  }

  function needs(list) {
    const panes = list.filter(r => r.pane);
    return {
      ...widths(list),
      tree: longest(panes.map(r => `${r.tree.join("")} `)),
      agent: Math.max(AGENT_WIDTH, longest(panes.map(r => r.pane.agent))),
    };
  }

  function columnLimits(room, gap, need) {
    const want = { branch: Math.min(need.branch, CAPS.branch), cwd: Math.min(need.cwd, CAPS.cwd) };
    if (!(room > 0)) return want;
    const cells = [need.tree, LABEL_ROOM, 1, need.agent, STATUS_WIDTH].concat(need.age ? [need.age] : []);
    const fixed = cells.reduce((n, w) => n + w, 0) + gap * (cells.length - 1);
    let shown = ["cwd", "branch"].filter(k => want[k] > 0);
    for (;;) {
      const space = Math.max(0, room - fixed - (gap + SLACK) * shown.length);
      const out = { branch: 0, cwd: 0 };
      for (const k of shown) out[k] = want[k];
      while (out.branch + out.cwd > space) {
        if (out.cwd >= out.branch) out.cwd--;
        else out.branch--;
      }
      const narrow = shown.find(k => out[k] < Math.min(want[k], CUT_MIN));
      if (!narrow) return out;
      shown = shown.filter(k => k !== narrow);
    }
  }

  function pageZoom(tree, session) {
    const sessions = ((tree && tree.workspaces) || []).flatMap(ws => ws.sessions || []);
    const id = String(session || "").toUpperCase();
    const own = (id && sessions.find(s => String(s.id).toUpperCase() === id)) || sessions.find(s => s.active);
    const page = own && (own.htmlOverlays || []).find(o => !o.pane && o.javascript);
    return page && page.zoom > 0 ? page.zoom : 1;
  }

  class Fit {
    constructor() {
      this.height = null;
    }

    next(cssHeight, zoom) {
      if (!(cssHeight > 0)) return null;
      const height = Math.ceil(cssHeight * (zoom > 0 ? zoom : 1) - 1e-3);
      if (height === this.height) return null;
      this.height = height;
      return height;
    }
  }

  function stackHeight(blocks, gap, padding) {
    return blocks.reduce((h, b) => h + b, 0) + gap * Math.max(0, blocks.length - 1) + padding;
  }

  class Panel {
    constructor(request, measure, session) {
      this.request = typeof request === "function" ? (...args) => request(...args) : null;
      this.measure = measure;
      this.session = session;
      this.fit = new Fit();
      this.fitted = null;
    }

    poll(apply) {
      if (!this.request) return Promise.resolve(null);
      return this.request("tree").then(r => {
        const tree = r && r.tree ? r.tree : r;
        return tree && tree.workspaces ? this.place(tree, apply(tree)) : null;
      });
    }

    place(tree, view) {
      if (!this.request) return null;
      const json = JSON.stringify(tree);
      if (json === this.fitted) return null;
      this.fitted = json;
      const height = this.fit.next(this.measure(view.all, view), pageZoom(tree, this.session));
      if (height !== null) this.request("session.overlay.resize", { args: { height } }).catch(() => {});
      return height;
    }
  }

  // A list key is the letter it types, or, on a layout that types no Latin letter, the one at its position.
  function letterOf(ev) {
    // the case comes from Shift, not from what Caps Lock makes the key type
    if (/^[A-Za-z]$/.test(ev.key || "")) return ev.shift ? ev.key.toUpperCase() : ev.key.toLowerCase();
    if (/^Key[A-Z]$/.test(ev.code || "")) return ev.shift ? ev.code.slice(3) : ev.code.slice(3).toLowerCase();
    if (ev.code === "Slash" && !ev.shift) return "/";
    return ev.key;
  }

  class Navigator {
    constructor(tree, branches, options = {}) {
      this.branches = branches || {};
      this.home = options.home || "";
      this.now = options.now || (() => Date.now() / 1000);
      this.query = "";
      this.searching = false;
      this.filter = null;
      this.room = null;
      this.gap = 0;
      this.setTree(tree);
      this.selected = this.currentTarget();
    }

    setTree(tree) {
      this.workspaces = workspacesOf(tree, this.branches, this.home);
    }

    setRoom(room, gap) {
      this.room = room;
      this.gap = gap || 0;
    }

    currentTarget() {
      for (const ws of this.workspaces) for (const p of ws.panes) if (p.current) return target.pane(p.session, p.pane);
      return null;
    }

    view() {
      const now = this.now();
      const limits = { ...columnLimits(this.room, this.gap, needs(rows(this.workspaces, "", null, now, WHOLE))),
                       line: this.room > 0 ? Math.floor(this.room) : Infinity };
      const list = rows(this.workspaces, this.query, this.filter, now, limits);
      const all = this.query || this.filter ? rows(this.workspaces, "", null, now, limits) : list;
      const selected = selectedIndex(list, this.selected);
      const search = this.searching || this.query ? `/ ${this.query}` : PLACEHOLDER;
      const count = list.reduce((n, r) => n + r.terminals, 0);
      const filter = this.filter && !this.searching ? this.filter : null;
      const key = filter ? [...HINTS.slice(0, HINTS.indexOf(FILTER_KEYS) + FILTER_KEYS.indexOf(filter))].length : -1;
      return {
        rows: list, all, selected, search, searching: this.searching,
        placeholder: search === PLACEHOLDER && !this.searching,
        filter: filter && FILTERS[filter],
        count: `${count} ${count === 1 ? "terminal" : "terminals"}`,
        hints: this.searching ? SEARCH_HINTS : HINTS,
        hintMarks: key < 0 ? [] : [[key, key + 1]],
        empty: NO_MATCH,
        widths: widths(all),
      };
    }

    step(delta) {
      const list = rows(this.workspaces, this.query, this.filter);
      const i = selectedIndex(list, this.selected);
      if (i === null) {
        this.selected = null;
        return;
      }
      this.selected = list[Math.max(0, Math.min(list.length - 1, i + delta))].target;
    }

    selectLast() {
      const list = rows(this.workspaces, this.query, this.filter);
      this.selected = list.length ? list[list.length - 1].target : null;
    }

    jumpWorkspace(forward) {
      const list = rows(this.workspaces, this.query, this.filter);
      const sel = selectedIndex(list, this.selected);
      if (sel === null) return;
      let section = sel;
      for (let i = sel; i >= 0; i--) {
        if (list[i].workspace) {
          section = i;
          break;
        }
      }
      const heads = [];
      for (let i = 0; i + 1 < list.length; i++) {
        if (list[i].workspace && !list[i + 1].workspace && (forward ? i > section : i < section)) heads.push(i);
      }
      const head = forward ? heads[0] : heads[heads.length - 1];
      if (head === undefined) return;
      const row = list.slice(head).find(r => r.pane);
      if (row) this.selected = row.target;
    }

    dropFilter() {
      const was = this.filter;
      this.filter = null;
      if (was !== "t") this.selected = null;
    }

    queryChanged() {
      this.filter = null;
      this.selected = null;
    }

    select(t) {
      this.selected = t;
    }

    openTarget() {
      const list = rows(this.workspaces, this.query, this.filter);
      const i = selectedIndex(list, this.selected);
      return i === null ? null : list[i].target;
    }

    // ev: {key, code, shift, ctrl}. Returns {handled}, plus {close: true} or {open: target}.
    key(ev) {
      const ctrl = ev.ctrl ? String(letterOf(ev)).toLowerCase() : null;
      if (ev.ctrl && !["n", "p", "u", "d"].includes(ctrl)) return { handled: false };
      if (ev.key === "Escape") {
        if (!this.searching) return { handled: true, close: true };
        this.searching = false;
        return { handled: true };
      }
      if (ev.key === "Enter") {
        const t = this.openTarget();
        return t ? { handled: true, open: t } : { handled: true };
      }
      return { handled: this.searching ? this.searchKey(ev, ctrl) : this.listKey(ev, ctrl) };
    }

    searchKey(ev, ctrl) {
      if (ev.key === "ArrowUp" || ctrl === "p") this.step(-1);
      else if (ev.key === "ArrowDown" || ctrl === "n") this.step(1);
      else if (ev.key === "Backspace") {
        if (this.query) {
          this.query = [...this.query].slice(0, -1).join("");
          this.queryChanged();
        }
      } else if (!ev.ctrl && [...ev.key].length === 1) {
        this.query += ev.key;
        this.queryChanged();
      } else return false;
      return true;
    }

    listKey(ev, ctrl) {
      const k = ev.ctrl ? null : letterOf(ev);
      if (ev.key === "ArrowUp" || k === "k") this.step(-1);
      else if (ev.key === "ArrowDown" || k === "j") this.step(1);
      else if (ctrl === "u" || ev.key === "PageUp") this.step(-PAGE);
      else if (ctrl === "d" || ev.key === "PageDown") this.step(PAGE);
      else if (ev.key === "ArrowLeft") this.jumpWorkspace(false);
      else if (ev.key === "ArrowRight") this.jumpWorkspace(true);
      else if (ev.key === "Home") this.selected = null;
      else if (ev.key === "End" || k === "G") this.selectLast();
      else if (ev.key === "Backspace") {
        if (this.filter) this.dropFilter();
      } else if (k === "/") {
        this.searching = true;
        this.filter = null;
      } else if (k === "a") {
        this.query = "";
        this.dropFilter();
      } else if (k && Object.hasOwn(FILTERS, k)) {
        this.query = "";
        this.filter = k;
        this.selected = k === "t" && this.selected ? topLevelTarget(this.workspaces, this.selected) : null;
      } else return false;
      return true;
    }
  }

  return { Navigator, Fit, Panel, agentOf, workspacesOf, rows, selectedIndex, target, fuzzy, parts, age, pageZoom,
           columnLimits, stackHeight };
})();

if (typeof module !== "undefined") module.exports = GoTo;
