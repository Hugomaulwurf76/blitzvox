// Aufnahmefenster – Nachbau von ui/workflow_window.py (Version 1.0.8).
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const kind = new URLSearchParams(location.search).get("kind") || "transcription";

const el = (id) => document.getElementById(id);
const recordBtn = el("record");
const copyBtn = el("copy");
const statusEl = el("status");
const resultEl = el("result");

// ── Wellenform (wie WaveformWidget) ─────────────────────────────────────────

const css = getComputedStyle(document.documentElement);
const COLOR = css.getPropertyValue("--primary").trim();
const IDLE = css.getPropertyValue("--hairline-strong").trim();
const canvas = el("waveform");
const ctx = canvas.getContext("2d");
let bars = new Array(20).fill(0);
let active = false;
let level = 0;
let timer = null;

function hexToRgb(hex) {
  const n = parseInt(hex.replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const n = bars.length;
  const spacing = 2;
  const barW = Math.max(2, Math.floor((w - n * 2) / n));
  const [r, g, b] = hexToRgb(COLOR);
  bars.forEach((lv, i) => {
    const barH = Math.max(3, Math.floor(lv * h * 0.9));
    const x = i * (barW + spacing);
    const y = Math.floor((h - barH) / 2);
    ctx.fillStyle = lv <= 0 ? IDLE : `rgba(${r},${g},${b},${0.35 + Math.min(lv, 1) * 0.65})`;
    ctx.beginPath();
    ctx.roundRect(x, y, barW, barH, 2);
    ctx.fill();
  });
}

function tick() {
  bars.shift();
  if (active) {
    const base = Math.max(0.05, level);
    bars.push(base * (0.5 + Math.random() * 0.5));
  } else {
    bars.push(0);
  }
  draw();
}

// Pegel alle 50 ms abfragen, solange die Aufnahme läuft
function startLevel() {
  if (timer) return;
  timer = setInterval(async () => {
    try {
      const lv = await invoke("wf_level", { kind });
      if (lv !== null) {
        active = true;
        level = lv;
      }
    } catch (_) {
      /* Fenster wird geschlossen */
    }
    tick();
  }, 50);
}

function stopLevel() {
  if (timer) clearInterval(timer);
  timer = null;
  active = false;
  bars = new Array(bars.length).fill(0);
  draw();
}

// ── Fenstergröße an den Inhalt anpassen (wie adjustSize) ────────────────────

function fit() {
  requestAnimationFrame(() => {
    const h = el("main").getBoundingClientRect().height;
    invoke("fit_height", { height: h }).catch(() => {});
  });
}

// ── Phasen (wie update_phase) ───────────────────────────────────────────────

function setStatus(text, role) {
  statusEl.textContent = text;
  statusEl.className = "status" + (role ? " " + role : "");
}

function setRecordButton(text, variant, enabled = true) {
  recordBtn.textContent = text;
  recordBtn.className = variant + " lg";
  recordBtn.disabled = !enabled;
}

// Bei „nichts gesagt“ schließt sich das Fenster nach 2 s von selbst (neu in 2.0,
// Entscheidung Nutzer 25.11.2026). Echte Fehler bleiben stehen, damit man sie lesen kann.
const AUTO_CLOSE = ["Keine Sprache erkannt.", "Aufnahme zu kurz — bitte länger sprechen."];
let closeTimer = null;

function updatePhase(phase, message, autoPaste) {
  if (closeTimer) {
    clearTimeout(closeTimer);
    closeTimer = null;
  }
  setStatus(message, "");
  switch (phase) {
    case "idle":
      stopLevel();
      setRecordButton("🎙  Aufnahme starten", "primary");
      setStatus("Bereit", "");
      break;
    case "recording":
      setRecordButton("⏹  Aufnahme stoppen", "stop");
      resultEl.classList.add("hidden");
      copyBtn.classList.add("hidden");
      startLevel();
      break;
    case "transcribing":
    case "rewriting":
      stopLevel();
      setRecordButton("⏳  Verarbeitung …", recordBtn.classList.contains("stop") ? "stop" : "primary", false);
      break;
    case "done":
      stopLevel();
      setRecordButton("🎙  Neue Aufnahme", "primary");
      resultEl.value = message;
      resultEl.classList.remove("hidden");
      copyBtn.classList.remove("hidden");
      // Zwischenablage und Einfügen erledigt das Programm
      setStatus(autoPaste ? "✅  Wird eingefügt …" : "✅  In Zwischenablage kopiert", "success");
      break;
    case "error":
      stopLevel();
      setRecordButton("🎙  Erneut versuchen", "primary");
      setStatus(`❌  ${message}`, "error");
      if (AUTO_CLOSE.includes(message)) {
        closeTimer = setTimeout(() => invoke("wf_cancel", { kind }), 2000);
      }
      break;
  }
  fit();
}

// ── Knöpfe ──────────────────────────────────────────────────────────────────

recordBtn.addEventListener("click", () => invoke("wf_toggle", { kind }));

copyBtn.addEventListener("click", async () => {
  const text = resultEl.value;
  if (!text) return;
  await invoke("copy_text", { text });
  copyBtn.textContent = "✅  Kopiert!";
  setTimeout(() => (copyBtn.textContent = "📋  Kopieren"), 2000);
});

el("cancel").addEventListener("click", () => invoke("wf_cancel", { kind }));

// ── Start ───────────────────────────────────────────────────────────────────

listen("wf-phase", (ev) => {
  const p = ev.payload;
  if (p.kind === kind) updatePhase(p.phase, p.message, p.autoPaste);
});

invoke("wf_info", { kind }).then((info) => {
  document.title = info.label;
  el("icon").textContent = info.icon;
  el("title").textContent = info.label;
  el("subtitle").textContent = info.subtitle;
  if (info.phase === "recording") {
    updatePhase("recording", "Aufnahme läuft …", false);
  } else {
    draw();
    fit();
  }
});
