// Korrekturfenster – Nachbau von ui/correction_window.py (Version 1.0.8).
//
// 1. Text erscheint im Eingabefeld und kann bearbeitet werden.
// 2. Während des Tippens werden Lern-Vorschläge („falsch -> richtig“) angezeigt.
// 3. Nur angehakte Vorschläge werden gespeichert (Standard: kein Haken).
// 4. „Übernehmen“ speichert die Haken und kopiert den Text in die Zwischenablage.
"use strict";

const { invoke } = window.__TAURI__.core;

const el = (id) => document.getElementById(id);
const editor = el("editor");
const box = el("suggestions");
const empty = el("empty");
const statusEl = el("status");

let original = "";
// Schlüssel "falsch\u0000richtig" -> Checkbox
let checks = new Map();
let debounce = null;

function fit() {
  requestAnimationFrame(() => {
    const h = el("main").getBoundingClientRect().height;
    invoke("fit_height", { height: h }).catch(() => {});
  });
}

async function refreshSuggestions() {
  const pairs = await invoke("corr_suggest", { original, edited: editor.value });
  // Haken-Zustand bestehender Vorschläge merken
  const oldState = new Map([...checks].map(([k, cb]) => [k, cb.checked]));
  for (const cb of checks.values()) cb.closest("label").remove();
  checks = new Map();

  for (const [wrong, right] of pairs) {
    const key = wrong + "\u0000" + right;
    const label = document.createElement("label");
    label.className = "check";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = oldState.get(key) || false;
    const span = document.createElement("span");
    span.textContent = `„${wrong}“  →  „${right}“`;
    span.style.whiteSpace = "pre";
    label.append(cb, span);
    box.append(label);
    checks.set(key, cb);
  }
  empty.classList.toggle("hidden", pairs.length > 0);
  fit();
}

editor.addEventListener("input", () => {
  clearTimeout(debounce);
  debounce = setTimeout(refreshSuggestions, 400);
});

el("apply").addEventListener("click", async () => {
  clearTimeout(debounce);
  await refreshSuggestions(); // sicher auf aktuellem Stand

  const learned = [...checks].filter(([, cb]) => cb.checked).map(([k]) => k.split("\u0000"));
  const text = editor.value;
  await invoke("corr_apply", { learned, text });
  original = text; // weitere Korrekturen bauen hierauf auf
  await refreshSuggestions();

  const n = learned.length;
  statusEl.textContent = n
    ? `✅  ${n} Korrektur${n !== 1 ? "en" : ""} gemerkt, Text in Zwischenablage.`
    : "✅  Text in Zwischenablage (nichts gemerkt).";
  statusEl.style.whiteSpace = "pre-wrap";
  fit();
});

el("close").addEventListener("click", () => invoke("close_window"));

invoke("corr_text").then((text) => {
  original = text;
  editor.value = text;
  fit();
});
