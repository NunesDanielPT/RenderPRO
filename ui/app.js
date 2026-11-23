// RenderPRO — interface. Toda a lógica de codecs/planeamento está no motor (Rust);
// aqui gere-se a fila, a janela de definições e o progresso.
"use strict";

const TAURI = window.__TAURI__;
const invoke = TAURI ? TAURI.core.invoke : mockInvoke;
const listen = TAURI ? TAURI.event.listen : async () => () => {};
const $ = (s, r = document) => r.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

// ---------------------------------------------------------------- estado

const DEFAULT_SETTINGS = {
  player: "playerslice", preset: "auto", quality: "high", size: "original", width: 1920, height: 1080, fit: "stretch",
  align: "pad", fps: null, audio: true, stereo: true, normalize: false, lufs: -16, deinterlace: true,
  output: "beside", folder: "", suffix: true, overwrite: false, verify: true, low_priority: true, gpu: true,
};
const DEFAULT_PREFS = { concurrency: 1, whenDone: "nothing", checkUpdates: true };

function load(key, def) {
  try { return { ...def, ...JSON.parse(localStorage.getItem(key) || "{}") }; } catch (_) { return { ...def }; }
}
function save(key, v) { try { localStorage.setItem(key, JSON.stringify(v)); } catch (_) {} }

const S = {
  items: [],
  settings: load("renderpro.settings", DEFAULT_SETTINGS),
  prefs: load("renderpro.prefs", DEFAULT_PREFS),
  rendering: false,
  stop: false,
  info: null,
  release: null,
  batch: { total: 0, done: 0 },
};
let nextId = 1;

const PLAYERS = {
  playerslice: { name: "PLAYERSLICE", color: "var(--ps)", sub: "psSub" },
  videohost: { name: "VIDEOHOST PLAYER", color: "var(--vh)", sub: "vhSub" },
  custom: { name: null, color: "var(--muted)", sub: "customSub" },
};
const playerName = (p) => PLAYERS[p].name || t("custom");
const gpuCaps = () => (S.info && S.info.gpu) || { h264: false, hevc: false };
const gpuFor = (codec, st) => st.gpu && ((codec === "h264" && gpuCaps().h264) || (codec === "h265" && gpuCaps().hevc));

// ---------------------------------------------------------------- formatação

function fmtDur(s) {
  if (!(s > 0)) return "—";
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
  return (h ? h + ":" + String(m).padStart(2, "0") : m) + ":" + String(x).padStart(2, "0");
}
function fmtFps(n, d) {
  if (!d) return "—";
  const f = n / d;
  return (Math.abs(f - Math.round(f)) < 0.005 ? Math.round(f) : f.toFixed(2)).toString();
}
function fmtBytes(b) {
  if (!(b > 0)) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return b.toFixed(i >= 3 ? 1 : 0) + " " + u[i];
}
const baseName = (p) => p.split(/[\\/]/).pop();
const dirName = (p) => p.slice(0, Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")));
const noteText = (n) => t(n.key, ...(n.args || []));

function errText(e) {
  const s = String(e ?? "");
  const i = s.indexOf(":");
  const head = i < 0 ? s : s.slice(0, i);
  const rest = i < 0 ? "" : s.slice(i + 1);
  switch (head) {
    case "plan": { const [k, a] = [rest.slice(0, rest.indexOf(":")), rest.slice(rest.indexOf(":") + 1)]; return t(k, ...a.split("|")); }
    case "nospace": return t("e.nospace", ...rest.split(":"));
    case "ffmpeg": return t("e.ffmpeg", rest.slice(rest.indexOf(":") + 1) || rest);
    case "verify": case "probe": case "spawn": case "rename": case "mkdir": return t("e." + head, rest);
    default: return I18N.pt["e." + head] ? t("e." + head) : s;
  }
}

function toast(msg, kind = "") {
  const el = document.createElement("div");
  el.className = "toast " + kind;
  el.textContent = msg;
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), 6000);
}

// ---------------------------------------------------------------- fila

async function addPaths(paths) {
  if (!paths || !paths.length) return;
  let files = [];
  try { files = await invoke("expand_paths", { paths }); } catch (_) { files = paths; }
  const known = new Set(S.items.filter((i) => !["done"].includes(i.status)).map((i) => i.path.toLowerCase()));
  const fresh = [];
  for (const f of files) {
    if (known.has(f.toLowerCase())) continue;
    known.add(f.toLowerCase());
    const it = { id: "j" + nextId++, path: f, name: baseName(f), status: "probing", info: null, plan: null, thumb: null };
    S.items.push(it);
    fresh.push(it);
  }
  renderList();
  // análise com 3 em paralelo
  const q = fresh.slice();
  await Promise.all([0, 1, 2].map(async () => {
    while (q.length) {
      const it = q.shift();
      try {
        it.info = await invoke("probe_file", { path: it.path });
        it.status = "ready";
      } catch (e) {
        it.status = "invalid";
        it.error = String(e) === "novideo" ? t("e.novideo") : errText(e);
      }
      updateRow(it);
      if (it.info) {
        invoke("thumbnail", { path: it.path, at: Math.min(it.info.duration * 0.3, 10) })
          .then((u) => { it.thumb = u; updateRow(it); })
          .catch(() => {});
      }
    }
  }));
  await replan();
}

async function replan() {
  const ready = S.items.filter((i) => i.info);
  if (!ready.length) { updateCounts(); return; }
  try {
    const plans = await invoke("plan_files", { infos: ready.map((i) => i.info), settings: S.settings });
    ready.forEach((it, k) => { if (!["rendering", "done"].includes(it.status)) it.plan = plans[k]; });
  } catch (e) { console.error(e); }
  ready.forEach(updateRow);
  updateCounts();
}

function removeItem(it) {
  if (it.status === "rendering") return;
  S.items = S.items.filter((x) => x !== it);
  renderList();
}

// ---------------------------------------------------------------- lista

const ICONS = {
  folder: '<svg viewBox="0 0 16 16" width="15" height="15"><path fill="none" stroke="currentColor" stroke-width="1.3" d="M1.5 3.5h4.5l1.5 1.5h7v8h-13z"/></svg>',
  play: '<svg viewBox="0 0 16 16" width="13" height="13"><path d="M4 2.5l9 5.5-9 5.5z" fill="currentColor"/></svg>',
  x: '<svg viewBox="0 0 12 12" width="11" height="11"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.4"/></svg>',
  stop: '<svg viewBox="0 0 12 12" width="11" height="11"><rect x="2" y="2" width="8" height="8" fill="currentColor"/></svg>',
  more: '<svg viewBox="0 0 16 16" width="15" height="15"><circle cx="3" cy="8" r="1.4" fill="currentColor"/><circle cx="8" cy="8" r="1.4" fill="currentColor"/><circle cx="13" cy="8" r="1.4" fill="currentColor"/></svg>',
};

function renderList() {
  const has = S.items.length > 0;
  $("#empty").classList.toggle("hidden", has);
  $("#list").classList.toggle("hidden", !has);
  const rows = $("#rows");
  rows.innerHTML = "";
  for (const it of S.items) {
    const el = document.createElement("div");
    el.className = "row item";
    el.dataset.id = it.id;
    rows.appendChild(el);
    updateRow(it);
  }
  updateCounts();
}

function rowEl(it) { return document.querySelector(`.row.item[data-id="${it.id}"]`); }

function stateHtml(it) {
  const p = it.plan;
  switch (it.status) {
    case "probing": return `<div class="l dim">${t("st_probing")}</div>`;
    case "invalid": return `<div class="l err" title="${esc(it.error)}">${t("st_invalid")}</div><div class="sub">${esc(it.error)}</div>`;
    case "ready":
      if (p && p.error) return `<div class="l err" title="${esc(noteText(p.error))}">${esc(noteText(p.error))}</div>`;
      return `<div class="l">${t("st_ready")}</div>`;
    case "queued": return `<div class="l dim">${t("st_queued")}</div><div class="bar"><div style="width:0"></div></div>`;
    case "rendering": {
      const ph = it.phase || "analyze";
      const pct = Math.max(0, Math.min(100, it.pct || 0));
      let sub = "";
      if (ph === "encode" || ph === "verify") {
        const bits = [pct.toFixed(0) + "%"];
        if (it.fps > 0) bits.push(Math.round(it.fps) + " fps");
        if (it.speed > 0) bits.push(it.speed.toFixed(1) + "×");
        if (it.eta > 1) bits.push(t("left") + " " + fmtDur(it.eta));
        sub = bits.join(" · ");
      } else if (ph === "loudness") sub = pct.toFixed(0) + "%";
      return `<div class="l">${t("st_" + ph)}</div><div class="bar ${ph === "verify" ? "verify" : ""}"><div style="width:${ph === "analyze" ? 0 : pct}%"></div></div><div class="sub">${sub}</div>`;
    }
    case "done": {
      const r = it.result;
      const w = r.warnings && r.warnings.length;
      return `<div class="l ${w ? "warn" : "ok"}">✓ ${t("st_done")}${r.verified ? " · " + t("verified") : ""}</div>` +
        `<div class="sub" title="${esc(r.output)}">${esc(baseName(r.output))} · ${fmtBytes(r.size)} · ${fmtDur(r.seconds)}${r.encoder === "nvenc" ? " · GPU" : ""}</div>` +
        (w ? `<div class="sub" style="color:var(--warn)" title="${esc(r.warnings.map(noteText).join("\n"))}">⚠ ${esc(noteText(r.warnings[r.warnings.length - 1]))}</div>` : "");
    }
    case "error": return `<div class="l err" title="${esc(it.error)}">✕ ${esc(it.error)}</div>`;
    case "cancelled": return `<div class="l dim">${t("st_cancelled")}</div>`;
  }
  return "";
}

function updateRow(it) {
  const el = rowEl(it);
  if (!el) return;
  const i = it.info;
  const p = it.plan;
  const tags = [];
  if (i) {
    if (i.has_alpha) tags.push(`<span class="tag acc">${t("alphaTag")}</span>`);
    if (i.vfr) tags.push(`<span class="tag warn">${t("vfrTag")}</span>`);
    if (i.interlaced) tags.push(`<span class="tag warn">${t("interlacedTag")}</span>`);
  }
  const outWarn = p && p.warnings.length ? p.warnings.map(noteText).join("\n") : "";
  const outTxt = p ? `<div class="c">${esc(p.codec_label)}${p.auto ? ' <span class="tag acc">auto</span>' : ""}</div>` +
    `<div class="w">${p.out_w}×${p.out_h} · ${fmtFps(p.fps_num, p.fps_den)} fps${p.warnings.length ? ' · <span style="color:var(--warn)">⚠ ' + p.warnings.length + "</span>" : ""}</div>` : "—";
  const canAct = !["rendering", "probing"].includes(it.status);
  el.innerHTML = `
    <div class="thumb ${i && i.has_alpha ? "alpha" : ""}">${it.thumb ? `<img src="${it.thumb}" alt="">` : ""}</div>
    <div class="name"><div class="n" title="${esc(it.path)}">${esc(it.name)}</div><div class="p">&lrm;${esc(dirName(it.path))}</div>${tags.length ? `<div class="tags">${tags.join("")}</div>` : ""}</div>
    <div class="cell" title="${esc(i ? i.codec_label + " " + i.pix_fmt : "")}">${i ? esc(i.codec_label) : "—"}${i && i.bit_depth > 8 ? ` <span class="dim">${i.bit_depth}b</span>` : ""}</div>
    <div class="cell mono">${i ? i.width + "×" + i.height : "—"}</div>
    <div class="cell mono">${i ? fmtFps(i.fps_num, i.fps_den) : "—"}</div>
    <div class="cell mono">${i ? fmtDur(i.duration) : "—"}</div>
    <div class="cell">${i ? (i.audio_index != null ? esc(i.audio_codec) + (i.audio_channels ? " " + i.audio_channels + "ch" : "") : `<span class="dim">${t("noAudio")}</span>`) : "—"}</div>
    <div class="cell out" title="${esc(outWarn)}">${outTxt}</div>
    <div class="state">${stateHtml(it)}</div>
    <div class="acts">
      ${it.status === "rendering" || it.status === "queued" ? `<button class="ic" data-a="cancel" title="${t("cancel")}">${ICONS.stop}</button>` : ""}
      ${it.status === "done" ? `<button class="ic" data-a="reveal" title="${t("reveal")}">${ICONS.folder}</button>` : ""}
      ${canAct ? `<button class="ic" data-a="menu">${ICONS.more}</button>` : ""}
    </div>`;
  el.querySelectorAll("[data-a]").forEach((b) => b.addEventListener("click", (ev) => { ev.stopPropagation(); rowAction(it, b.dataset.a, ev); }));
  el.oncontextmenu = (ev) => { ev.preventDefault(); if (canAct) rowAction(it, "menu", ev); };
  el.ondblclick = () => { if (it.status === "done") invoke("open_path", { path: it.result.output }); };
}

function rowAction(it, a, ev) {
  if (a === "cancel") {
    if (it.status === "queued") { it.status = "ready"; updateRow(it); updateCounts(); }
    else invoke("cancel_job", { id: it.id });
  } else if (a === "reveal") invoke("reveal", { path: it.result.output });
  else if (a === "menu") {
    const items = [];
    if (it.status === "done") {
      items.push([t("play"), () => invoke("open_path", { path: it.result.output })]);
      items.push([t("reveal"), () => invoke("reveal", { path: it.result.output })]);
      items.push([t("retry"), () => { it.status = "ready"; it.result = null; replan(); }]);
    }
    if (["error", "cancelled"].includes(it.status)) items.push([t("retry"), () => { it.status = "ready"; it.error = null; replan(); }]);
    items.push([t("reveal") + " (original)", () => invoke("reveal", { path: it.path })]);
    items.push([t("remove"), () => removeItem(it)]);
    showMenu(ev, items);
  }
}

function showMenu(ev, items) {
  const m = $("#menu");
  m.innerHTML = "";
  for (const [label, fn] of items) {
    const b = document.createElement("button");
    b.textContent = label;
    b.onclick = () => { hideMenu(); fn(); };
    m.appendChild(b);
  }
  m.classList.remove("hidden");
  const r = m.getBoundingClientRect();
  m.style.left = Math.min(ev.clientX, innerWidth - r.width - 8) + "px";
  m.style.top = Math.min(ev.clientY, innerHeight - r.height - 8) + "px";
}
function hideMenu() { $("#menu").classList.add("hidden"); }
document.addEventListener("click", (e) => { if (!$("#menu").contains(e.target)) hideMenu(); });

function updateCounts() {
  const n = S.items.length;
  $("#count").textContent = n ? t("files", n) + " · " + fmtDur(S.items.reduce((a, i) => a + (i.info ? i.info.duration : 0), 0)) : "";
  const pl = PLAYERS[S.settings.player];
  $("#target-name").textContent = playerName(S.settings.player);
  $("#target-dot").style.background = pl.color;
  $("#target-dot").style.color = pl.color;
  const btn = $("#btn-render");
  btn.classList.toggle("stop", S.rendering);
  $("#render-label").textContent = S.rendering ? t("stopAll") : t("render");
  btn.disabled = !S.rendering && !S.items.some((i) => i.info);
  if (!S.rendering) $("#total-fill").style.width = "0";
}

// ---------------------------------------------------------------- render

function renderable(it) {
  return it.info && ["ready", "error", "cancelled"].includes(it.status) && !(it.plan && it.plan.error);
}

async function startRender(items) {
  if (!items.length) { toast(t("nothingToRender")); return; }
  for (const it of items) { it.status = "queued"; it.error = null; it.result = null; it.pct = 0; updateRow(it); }
  if (S.rendering) return; // já está a correr: entram na fila
  S.rendering = true;
  S.stop = false;
  S.batch = { total: items.length, done: 0, ok: 0, err: 0, outDirs: new Set() };
  updateCounts();
  const worker = async () => {
    while (!S.stop) {
      const it = S.items.find((x) => x.status === "queued");
      if (!it) break;
      it.status = "rendering";
      it.phase = "analyze";
      it.pct = 0;
      updateRow(it);
      updateStatus();
      try {
        it.result = await invoke("render_file", { id: it.id, path: it.path, settings: S.settings });
        it.status = "done";
        S.batch.ok++;
        S.batch.outDirs.add(dirName(it.result.output));
      } catch (e) {
        const s = String(e);
        it.status = s === "cancelled" ? "cancelled" : "error";
        it.error = errText(e);
        if (it.status === "error") S.batch.err++;
      }
      S.batch.done++;
      updateRow(it);
      updateStatus();
    }
  };
  const n = Math.max(1, Math.min(2, S.prefs.concurrency | 0));
  await Promise.all(Array.from({ length: n }, worker));
  S.items.filter((x) => x.status === "queued").forEach((x) => { x.status = "ready"; updateRow(x); });
  S.rendering = false;
  updateCounts();
  const msg = t("statusDone", S.batch.ok, S.batch.err);
  $("#status").textContent = msg;
  if (S.batch.ok || S.batch.err) toast(msg, S.batch.err ? "err" : "ok");
  if (S.prefs.whenDone === "open" && S.batch.outDirs.size === 1) invoke("open_path", { path: [...S.batch.outDirs][0] });
}

function updateStatus() {
  if (!S.rendering) return;
  const b = S.batch;
  const cur = S.items.filter((x) => x.status === "rendering");
  const queued = S.items.filter((x) => x.status === "queued").length;
  b.total = b.done + cur.length + queued;
  $("#status").textContent = t("statusRendering", Math.min(b.done + 1, b.total), b.total);
  const frac = (b.done + cur.reduce((a, x) => a + (x.phase === "verify" ? 0.8 + (x.pct || 0) / 500 : x.phase === "encode" ? (x.pct || 0) / 125 : 0), 0)) / Math.max(1, b.total);
  $("#total-fill").style.width = (frac * 100).toFixed(1) + "%";
}

function stopAll() {
  S.stop = true;
  S.items.filter((x) => x.status === "queued").forEach((x) => { x.status = "ready"; updateRow(x); });
  invoke("cancel_all");
}

// ---------------------------------------------------------------- janela de render

let draft = null;
let planTimer = null;

function openRender() {
  if (S.rendering) { stopAll(); return; }
  if (!S.items.some((i) => i.info)) { toast(t("nothingToRender")); return; }
  draft = JSON.parse(JSON.stringify(S.settings));
  openModal("render-box", `
    <div class="m-head">${t("renderTitle")}<div class="spacer"></div><span class="dim" style="font-weight:400;font-size:12.5px" id="r-count"></span></div>
    <div class="render-grid">
      <div class="left" id="r-left"></div>
      <div class="right"><div class="sec-t">${t("summary")}</div><div id="r-sum"></div></div>
    </div>
    <div class="m-foot">
      <div class="totals" id="r-totals"></div>
      <div class="spacer"></div>
      <button class="btn" id="r-cancel">${t("cancel")}</button>
      <button class="btn-render" id="r-go"></button>
    </div>`);
  $("#r-cancel").onclick = closeModal;
  $("#r-go").onclick = () => {
    S.settings = draft;
    save("renderpro.settings", S.settings);
    closeModal();
    replan().then(() => startRender(S.items.filter(renderable)));
  };
  drawRenderLeft();
  schedulePlan(0);
}

async function drawRenderLeft() {
  const d = draft;
  let codecList = [];
  try { codecList = await invoke("codecs", { player: d.player }); } catch (_) {}
  if (d.preset !== "auto" && !codecList.some((c) => c.id === d.preset)) d.preset = "auto";
  const seg = (key, opts) => `<div class="seg" data-k="${key}">${opts.map(([v, l]) => `<button data-v="${v}" class="${String(d[key]) === String(v) ? "on" : ""}">${l}</button>`).join("")}</div>`;
  const chk = (key, label) => `<label class="chk"><input type="checkbox" data-c="${key}" ${d[key] ? "checked" : ""}><span>${label}</span></label>`;
  const usesQuality = ["h264", "h265"].includes(d.preset) || (d.preset === "auto" && d.player === "videohost");
  const fpsOpts = ["", "23.976", "24", "25", "29.97", "30", "50", "59.94", "60"];
  $("#r-left").innerHTML = `
    <div class="sec"><div class="sec-t">${t("player")}</div><div class="players">
      ${Object.entries(PLAYERS).map(([k, p]) => `<button class="pcard ${d.player === k ? "on" : ""}" data-player="${k}" style="--pc:${p.color}">
        <div class="pn"><span class="dot" style="background:${p.color}"></span>${esc(playerName(k))}</div><div class="ps">${t(p.sub)}</div></button>`).join("")}
    </div></div>
    <div class="sec"><div class="sec-t">${t("codec")}</div><div class="codecs">
      <button class="copt ${d.preset === "auto" ? "on" : ""}" data-preset="auto"><span class="rd"></span><span><div class="ct">${t("auto")}</div></span></button>
      ${codecList.map((c) => `<button class="copt ${d.preset === c.id ? "on" : ""}" data-preset="${c.id}"><span class="rd"></span><span><div class="ct">${c.label}</div><div class="cd">${t("desc." + c.id)}</div></span></button>`).join("")}
    </div></div>
    ${usesQuality ? `<div class="sec"><div class="sec-t">${t("quality")} · H.264/H.265</div>${seg("quality", [["normal", t("qNormal")], ["high", t("qHigh")], ["max", t("qMax")]])}
      ${gpuCaps().h264 || gpuCaps().hevc ? chk("gpu", t("gpuOpt")) : ""}</div>` : ""}
    <div class="sec"><div class="sec-t">${t("size")}</div>
      ${seg("size", [["original", t("sOriginal")], ["half", t("sHalf")], ["custom", t("sCustom")]])}
      ${d.size === "custom" ? `<div class="line"><input class="num" type="number" min="16" max="16384" data-n="width" value="${d.width}"> × <input class="num" type="number" min="16" max="16384" data-n="height" value="${d.height}">
        ${seg("fit", [["stretch", t("fitStretch")], ["fit", t("fitFit")], ["fill", t("fitFill")]])}</div>` : ""}
      <div class="sec-t" style="margin-top:12px">${t("align")}</div>
      ${seg("align", [["pad", t("alignPad")], ["scale", t("alignScale")]])}
    </div>
    <div class="sec"><div class="sec-t">${t("fps")}</div>
      <select class="sel" id="r-fps">${fpsOpts.map((f) => `<option value="${f}" ${String(d.fps || "") === f ? "selected" : ""}>${f || t("fpsOriginal")}</option>`).join("")}</select>
      ${d.player === "playerslice" ? `<div class="hint">${t("fpsHint")}</div>` : ""}
    </div>
    <div class="sec"><div class="sec-t">${t("audio")}</div>
      ${seg("audio", [[true, t("audioKeep")], [false, t("audioRemove")]])}
      ${d.audio ? chk("stereo", t("stereo")) + chk("normalize", t("normalize")) +
        (d.normalize ? `<div class="line"><span class="dim">${t("lufs")}</span><select class="sel" id="r-lufs">${[-23, -18, -16, -14].map((v) => `<option ${d.lufs === v ? "selected" : ""} value="${v}">${v} LUFS</option>`).join("")}</select></div>` : "") : ""}
    </div>
    <div class="sec"><div class="sec-t">${t("output")}</div>
      ${seg("output", [["beside", t("outBeside")], ["subfolder", t("outSub")], ["folder", t("outFolder")]])}
      ${d.output === "folder" ? `<div class="line"><input class="txt" id="r-folder" value="${esc(d.folder)}" readonly><button class="btn" id="r-pick">${t("choose")}</button></div>` : ""}
      ${chk("suffix", t("suffix"))}${chk("overwrite", t("overwrite"))}${chk("verify", t("verify"))}
    </div>
    <details class="sec"><summary>${t("advanced")}</summary>${chk("deinterlace", t("deinterlace"))}${chk("low_priority", t("lowPriority"))}</details>`;

  const L = $("#r-left");
  L.querySelectorAll("[data-player]").forEach((b) => (b.onclick = () => { d.player = b.dataset.player; d.preset = "auto"; drawRenderLeft(); schedulePlan(); }));
  L.querySelectorAll("[data-preset]").forEach((b) => (b.onclick = () => { d.preset = b.dataset.preset; drawRenderLeft(); schedulePlan(); }));
  L.querySelectorAll(".seg[data-k]").forEach((sg) => sg.querySelectorAll("button").forEach((b) => (b.onclick = () => {
    const k = sg.dataset.k;
    const v = b.dataset.v;
    d[k] = v === "true" ? true : v === "false" ? false : v;
    drawRenderLeft();
    schedulePlan();
  })));
  L.querySelectorAll("[data-c]").forEach((c) => (c.onchange = () => { d[c.dataset.c] = c.checked; if (c.dataset.c === "normalize") drawRenderLeft(); schedulePlan(); }));
  L.querySelectorAll("[data-n]").forEach((n) => (n.oninput = () => { d[n.dataset.n] = Math.max(16, Math.min(16384, parseInt(n.value, 10) || 16)); schedulePlan(); }));
  const fps = $("#r-fps"); if (fps) fps.onchange = () => { d.fps = fps.value || null; schedulePlan(); };
  const lufs = $("#r-lufs"); if (lufs) lufs.onchange = () => { d.lufs = parseFloat(lufs.value); schedulePlan(); };
  const pick = $("#r-pick");
  if (pick) pick.onclick = async () => {
    const dir = await openDialog({ directory: true, multiple: false });
    if (dir) { d.folder = Array.isArray(dir) ? dir[0] : dir; drawRenderLeft(); schedulePlan(); }
  };
}

function schedulePlan(ms = 150) {
  clearTimeout(planTimer);
  planTimer = setTimeout(drawSummary, ms);
}

async function drawSummary() {
  if (!draft || !$("#r-sum")) return;
  const list = S.items.filter((i) => i.info && ["ready", "error", "cancelled"].includes(i.status));
  let plans = [];
  try { plans = await invoke("plan_files", { infos: list.map((i) => i.info), settings: draft }); } catch (e) { console.error(e); }
  if (!$("#r-sum")) return;
  let total = 0, ok = 0, bad = 0;
  $("#r-sum").innerHTML = list.map((it, k) => {
    const p = plans[k];
    if (!p) return "";
    if (p.error) bad++; else { ok++; total += p.est_bytes; }
    return `<div class="sum-item ${p.error ? "bad" : ""}">
      <div class="sum-top"><span class="n">${esc(it.name)}</span><span class="sum-arrow">→</span><span class="sum-codec">${esc(p.codec_label)}</span>
        ${p.auto ? `<span class="tag acc">${t("recommended")}</span>` : p.codec === p.recommended ? `<span class="tag acc">${t("recommended")}</span>` : ""}</div>
      <div class="sum-spec">${it.info.width}×${it.info.height} ${fmtFps(it.info.fps_num, it.info.fps_den)}fps ${esc(it.info.codec_label)} → ${p.out_w}×${p.out_h} ${fmtFps(p.fps_num, p.fps_den)}fps${p.chunks ? " · " + p.chunks + " chunks" : ""}${["h264", "h265"].includes(p.codec) ? " · " + (gpuFor(p.codec, draft) ? "GPU NVENC" : "CPU") : ""}${p.audio ? " · " + (p.codec_label.startsWith("H.26") ? "AAC" : "PCM") + " 48 kHz" : ""} · ~${fmtBytes(p.est_bytes)}</div>
      ${p.auto || p.codec === p.recommended ? `<div class="sum-why">${esc(noteText(p.why))}</div>` : ""}
      ${p.warnings.map((w) => `<div class="sum-w">⚠ ${esc(noteText(w))}</div>`).join("")}
      ${p.error ? `<div class="sum-e">✕ ${esc(noteText(p.error))}</div>` : ""}
      <div class="sum-out" title="${esc(p.output)}">&lrm;${esc(p.output)}</div>
    </div>`;
  }).join("");
  let freeTxt = "";
  if (plans.length && plans[0]) {
    try {
      const dir = dirName(plans[0].output);
      const free = await invoke("free_space", { path: dir });
      if (free != null) freeTxt = ` · <span class="${free < total * 1.1 ? "" : ""}">${fmtBytes(free)} ${t("free")} ${esc(dir.slice(0, 3))}</span>` + (free < total * 1.1 ? ` <b style="color:var(--danger)">⚠ ${t("noSpace")}</b>` : "");
    } catch (_) {}
  }
  $("#r-totals").innerHTML = `<span>${t("estimate")}: <b>${fmtBytes(total)}</b>${freeTxt}</span>` + (bad ? `<span style="color:var(--danger)">${t("blocked", bad)}</span>` : "");
  $("#r-count").textContent = t("files", list.length);
  const go = $("#r-go");
  go.innerHTML = `<svg viewBox="0 0 16 16" width="13" height="13"><path d="M4 2.5l9 5.5-9 5.5z" fill="currentColor"/></svg> ${t("startRender", ok)}`;
  go.disabled = ok === 0;
}

// ---------------------------------------------------------------- modais

function openModal(cls, html) {
  const box = $("#modal-box");
  box.className = "modal-box " + cls;
  box.innerHTML = html;
  $("#modal").classList.remove("hidden");
}
function closeModal() { $("#modal").classList.add("hidden"); $("#modal-box").innerHTML = ""; draft = null; }
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("#modal").classList.contains("hidden")) closeModal(); });

function confirmBox(title, text, okLabel, onOk, danger = true) {
  openModal("small-box", `<div class="m-head">${esc(title)}</div><div class="m-body">${esc(text)}</div>
    <div class="m-foot"><div class="spacer"></div><button class="btn" id="c-no">${t("keepOpen")}</button><button class="btn ${danger ? "danger" : "primary"}" id="c-yes">${esc(okLabel)}</button></div>`);
  $("#c-no").onclick = closeModal;
  $("#c-yes").onclick = () => { closeModal(); onOk(); };
}

// ---------------------------------------------------------------- definições

function openSettings(tab = "general") {
  openModal("settings-box", `<div class="m-head">${t("settings")}<div class="spacer"></div><button class="ic" id="s-close">${ICONS.x}</button></div>
    <div class="set-grid"><div class="tabs" id="s-tabs">
      ${[["general", "tabGeneral"], ["guide", "tabGuide"], ["updates", "tabUpdates"], ["about", "tabAbout"]].map(([k, l]) => `<button data-t="${k}" class="${tab === k ? "on" : ""}">${t(l)}</button>`).join("")}
    </div><div class="tab-body" id="s-body"></div></div>`);
  $("#s-close").onclick = closeModal;
  $("#s-tabs").querySelectorAll("button").forEach((b) => (b.onclick = () => openSettings(b.dataset.t)));
  const B = $("#s-body");
  if (tab === "general") {
    B.innerHTML = `
      <div class="field"><label>${t("language")}</label><div class="seg" id="s-lang"><button data-v="pt" class="${LANG === "pt" ? "on" : ""}">Português</button><button data-v="en" class="${LANG === "en" ? "on" : ""}">English</button></div></div>
      <div class="field"><label>${t("concurrency")}</label><div class="seg" id="s-conc"><button data-v="1" class="${S.prefs.concurrency == 1 ? "on" : ""}">1</button><button data-v="2" class="${S.prefs.concurrency == 2 ? "on" : ""}">2</button></div></div>
      <div class="field"><label>${t("whenDone")}</label><select class="sel" id="s-done"><option value="nothing">${t("doneNothing")}</option><option value="open" ${S.prefs.whenDone === "open" ? "selected" : ""}>${t("doneOpen")}</option></select></div>
      <div class="field"><label>${t("gpuLabel")}</label><span class="${gpuCaps().h264 ? "" : "dim"}">${gpuCaps().h264 && gpuCaps().hevc ? t("gpuYes") : gpuCaps().h264 ? t("gpuPart") : t("gpuNo")}</span></div>
      <div class="field"><label></label><button class="btn" id="s-logs">${t("logs")}</button></div>`;
    $("#s-lang").querySelectorAll("button").forEach((b) => (b.onclick = () => { LANG = b.dataset.v; try { localStorage.setItem("renderpro.lang", LANG); } catch (_) {} applyI18n(); renderList(); openSettings("general"); }));
    $("#s-conc").querySelectorAll("button").forEach((b) => (b.onclick = () => { S.prefs.concurrency = +b.dataset.v; save("renderpro.prefs", S.prefs); openSettings("general"); }));
    $("#s-done").onchange = (e) => { S.prefs.whenDone = e.target.value; save("renderpro.prefs", S.prefs); };
    $("#s-logs").onclick = () => invoke("open_logs");
  } else if (tab === "guide") {
    const row = (c, w) => `<tr><td><b>${c}</b></td><td>${w}</td></tr>`;
    B.innerHTML = `<p>${t("guideIntro")}</p>
      <div class="sec-t">PLAYERSLICE</div><table class="guide-t"><tbody>
        ${row("HAP", t("desc.hap"))}${row("HAP Q", t("desc.hap_q"))}${row("HAP Alpha", t("desc.hap_alpha"))}
        ${row("H.264 / H.265", t("warn.ps_longgop", "H.264/H.265"))}${row("ProRes / DNxHR", t("warn.ps_cpu", "ProRes/DNxHR"))}
      </tbody></table>
      <div class="sec-t">VIDEOHOST PLAYER</div><table class="guide-t"><tbody>
        ${row("H.264", t("why.vh_h264"))}${row("HAP", t("why.vh_heavy"))}${row("HAP Alpha", t("why.vh_alpha"))}${row("H.265", t("warn.vh_h265"))}
      </tbody></table>
      <p class="dim">${t("guideRes")}<br>${t("guideFps")}<br>${t("guideAudio")}<br>${t("guideDisk")}</p>`;
  } else if (tab === "updates") {
    B.innerHTML = `<div class="field"><label>${t("version")}</label><b>${esc(S.info ? S.info.version : "—")}</b></div>
      <label class="chk"><input type="checkbox" id="s-auto" ${S.prefs.checkUpdates ? "checked" : ""}><span>${t("checkOnStart")}</span></label>
      <div class="line" style="margin-top:14px"><button class="btn" id="s-check">${t("checkUpdates")}</button><span id="s-upd" class="dim"></span></div>`;
    $("#s-auto").onchange = (e) => { S.prefs.checkUpdates = e.target.checked; save("renderpro.prefs", S.prefs); };
    $("#s-check").onclick = () => checkUpdates(true);
  } else {
    B.innerHTML = `<div style="display:flex;gap:16px;align-items:center"><img class="about-logo" src="logo-big.png" alt="">
      <div><div class="brand-name" style="font-size:24px">RENDER<b>PRO</b></div><div class="dim">${t("version")} ${esc(S.info ? S.info.version : "")}</div></div></div>
      <p>${t("aboutText")}</p>
      <div class="credit">${t("createdBy")} <b>Daniel Nunes</b><br><a class="link" id="s-gh">github.com/NunesDanielPT</a></div>
      <p class="dim" style="font-size:12px;margin-top:14px">${t("ffmpegNote")} ${S.info && S.info.ffmpeg ? "FFmpeg " + esc(S.info.ffmpeg) : ""} — <a class="link" id="s-ff">ffmpeg.org</a></p>`;
    $("#s-gh").onclick = () => invoke("open_url", { url: "https://github.com/NunesDanielPT" });
    $("#s-ff").onclick = () => invoke("open_url", { url: "https://ffmpeg.org/legal.html" });
  }
}

// ---------------------------------------------------------------- atualizações

function newer(a, b) {
  const pa = String(a).split(".").map((x) => parseInt(x, 10) || 0), pb = String(b).split(".").map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < 3; i++) { if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0); }
  return false;
}

async function checkUpdates(manual) {
  const out = $("#s-upd");
  try {
    const r = await invoke("latest_release");
    if (S.info && newer(r.version, S.info.version) && r.url) {
      S.release = r;
      showUpdateBanner();
      if (out) out.textContent = t("updateAvailable", r.version);
    } else if (out) out.textContent = t("upToDate");
  } catch (e) {
    if (manual && out) out.textContent = t("updateError");
  }
}

function showUpdateBanner() {
  const r = S.release;
  const b = $("#banner");
  b.innerHTML = `<span>${esc(t("updateAvailable", r.version))}</span><div class="spacer"></div>
    <button class="btn primary" id="u-now">${t("updateNow")}</button><button class="btn ghost" id="u-later">${t("updateLater")}</button>`;
  b.classList.remove("hidden");
  $("#u-later").onclick = () => b.classList.add("hidden");
  $("#u-now").onclick = async () => {
    if (S.rendering) { toast(t("e.busy"), "err"); return; }
    b.innerHTML = `<span>${t("updating")}</span><span id="u-pct" class="dim"></span>`;
    try { await invoke("self_update", { url: r.url, size: r.size }); } catch (e) { b.classList.add("hidden"); toast(errText(e), "err"); }
  };
}

// ---------------------------------------------------------------- diálogos de ficheiros

async function openDialog(opts) {
  if (!TAURI) return null;
  if (TAURI.dialog && TAURI.dialog.open) return TAURI.dialog.open(opts);
  return invoke("plugin:dialog|open", { options: opts });
}

async function pickFiles() {
  const r = await openDialog({ multiple: true, filters: [{ name: "Vídeo", extensions: ["mov", "mp4", "m4v", "mkv", "avi", "mxf", "webm", "wmv", "mpg", "mpeg", "m2ts", "mts", "ts", "flv", "gif", "dv"] }, { name: "*", extensions: ["*"] }] });
  if (r) addPaths(Array.isArray(r) ? r : [r]);
}
async function pickFolder() {
  const r = await openDialog({ directory: true, multiple: true });
  if (r) addPaths(Array.isArray(r) ? r : [r]);
}

// ---------------------------------------------------------------- arranque

function targetMenu(ev) {
  showMenu(ev, Object.keys(PLAYERS).map((k) => [playerName(k), () => {
    S.settings.player = k;
    S.settings.preset = "auto";
    save("renderpro.settings", S.settings);
    replan();
  }]));
}

async function init() {
  applyI18n();
  const win = TAURI ? TAURI.window.getCurrentWindow() : null;
  $("#btn-min").onclick = () => win && win.minimize();
  $("#btn-max").onclick = () => win && win.toggleMaximize();
  $("#btn-close").onclick = () => {
    if (S.rendering) confirmBox(t("closeTitle"), t("closeText"), t("closeAnyway"), () => invoke("force_quit"));
    else win && win.close();
  };
  $("#btn-settings").onclick = () => openSettings();
  $("#btn-add").onclick = $("#btn-add2").onclick = pickFiles;
  $("#btn-folder").onclick = $("#btn-folder2").onclick = pickFolder;
  $("#btn-clear-done").onclick = () => { S.items = S.items.filter((i) => i.status !== "done"); renderList(); };
  $("#btn-clear-all").onclick = () => { S.items = S.items.filter((i) => ["rendering", "queued"].includes(i.status)); renderList(); };
  $("#btn-render").onclick = openRender;
  $("#btn-target").onclick = (e) => { e.stopPropagation(); if (!S.rendering) targetMenu(e); };
  $("#modal").addEventListener("mousedown", (e) => { if (e.target.id === "modal" && $("#modal-box").classList.contains("small-box")) closeModal(); });

  await listen("renderpro://job", (e) => {
    const p = e.payload;
    const it = S.items.find((x) => x.id === p.id);
    if (!it || it.status !== "rendering") return;
    Object.assign(it, { phase: p.phase === "done" ? "verify" : p.phase, pct: p.pct, fps: p.fps, speed: p.speed, eta: p.eta });
    updateRow(it);
    updateStatus();
  });
  await listen("renderpro://close-requested", () => confirmBox(t("closeTitle"), t("closeText"), t("closeAnyway"), () => invoke("force_quit")));
  await listen("renderpro://open-files", (e) => addPaths(e.payload));
  await listen("renderpro://update", (e) => { const el = $("#u-pct"); if (el) el.textContent = " " + Math.round((e.payload[0] / Math.max(1, e.payload[1])) * 100) + "%"; });
  const ov = $("#drop-overlay");
  await listen("tauri://drag-enter", () => ov.classList.remove("hidden"));
  await listen("tauri://drag-over", () => ov.classList.remove("hidden"));
  await listen("tauri://drag-leave", () => ov.classList.add("hidden"));
  await listen("tauri://drag-drop", (e) => { ov.classList.add("hidden"); addPaths(e.payload.paths || []); });
  document.addEventListener("dragover", (e) => e.preventDefault());
  document.addEventListener("drop", (e) => e.preventDefault());

  try { S.info = await invoke("app_info"); } catch (_) {}
  $("#ffver").textContent = S.info && S.info.ffmpeg ? "FFmpeg " + S.info.ffmpeg : "";
  if (S.info && !S.info.ffmpeg) $("#status").textContent = t("ffmpegMissing");
  else $("#status").textContent = t("statusIdle");
  renderList();
  try { const f = await invoke("startup_files"); if (f && f.length) addPaths(f); } catch (_) {}
  if (S.prefs.checkUpdates) setTimeout(() => checkUpdates(false), 2500);
}

// ---------------------------------------------------------------- modo de pré-visualização (browser, sem Tauri)

async function mockInvoke(cmd, a) {
  const info = (name, w, h, fps, codec, extra = {}) => ({ path: "D:\\Show\\Conteúdos\\" + name, size: 1e9, duration: 63.2, video_index: 0, codec: "x",
    codec_label: codec, codec_tag: "", profile: "", width: w, height: h, pix_fmt: "yuv420p", bit_depth: 8, has_alpha: false, fps_num: fps, fps_den: 1,
    vfr: false, interlaced: false, rotation: 0, sar_num: 1, sar_den: 1, color_space: "", color_transfer: "", color_range: "", hdr: false, frames: 0, bitrate: 0,
    audio_index: 1, audio_codec: "aac", audio_channels: 2, audio_rate: 48000, audio_streams: 1, ...extra });
  const codecs = { playerslice: ["hap", "hap_q", "hap_alpha", "h264", "h265", "prores_hq", "prores_4444", "dnxhr_hq"], videohost: ["h264", "hap", "hap_q", "hap_alpha", "prores_hq", "h265"], custom: ["hap", "hap_alpha", "hap_q", "h264", "h265", "prores_hq", "prores_4444", "dnxhr_hq"] };
  const label = { hap: "HAP", hap_alpha: "HAP Alpha", hap_q: "HAP Q", h264: "H.264", h265: "H.265", prores_hq: "ProRes 422 HQ", prores_4444: "ProRes 4444", dnxhr_hq: "DNxHR HQ" };
  switch (cmd) {
    case "app_info": return { version: "0.2.0", windows: true, ffmpeg: "n9.0", bundled: true, gpu: { h264: true, hevc: true } };
    case "startup_files": return ["a", "b", "c", "d"];
    case "expand_paths": return a.paths;
    case "probe_file": return { a: info("LED_CENTER_T.mov", 4864, 936, 50, "ProRes 422 HQ"), b: info("Logo_loop.mov", 1080, 1080, 50, "ProRes 4444", { has_alpha: true, audio_index: null }), c: info("Abertura_4K.mp4", 3840, 2160, 25, "H.264", { vfr: true }), d: info("Entrevista.mxf", 1920, 1080, 25, "DNxHD", { interlaced: true }) }[a.path];
    case "thumbnail": throw "x";
    case "codecs": return codecs[a.player].map((id) => ({ id, label: label[id], alpha: id.includes("alpha") || id.includes("4444") }));
    case "plan_files": return a.infos.map((i) => {
      const c = a.settings.preset !== "auto" ? a.settings.preset : i.has_alpha ? "hap_alpha" : a.settings.player === "videohost" && i.width * i.height > 2.2e6 ? "hap" : a.settings.player === "videohost" ? "h264" : "hap";
      const w = Math.ceil(i.width / 4) * 4, h = Math.ceil(i.height / 4) * 4;
      return { codec: c, codec_label: label[c], recommended: c, auto: a.settings.preset === "auto", why: { key: i.has_alpha ? "why.ps_alpha" : "why.ps_hap", args: [] }, out_w: w, out_h: h,
        fps_num: i.fps_num, fps_den: 1, duration: i.duration, frames: 0, audio: i.audio_index != null, output: i.path.replace(/\.\w+$/, "_" + label[c].replace(/[ .]/g, "").toUpperCase() + ".mov"),
        est_bytes: w * h * 0.5 * 3000, chunks: 8, warnings: i.vfr ? [{ key: "warn.vfr", args: ["25.000"] }] : i.interlaced ? [{ key: "warn.deinterlaced", args: [] }] : [], error: null };
    });
    case "free_space": return 812e9;
    case "latest_release": return { version: "0.1.0" };
    default: return null;
  }
}

init();
