// Einstellungsfenster – Nachbau von ui/settings_window.py (Version 1.0.8).
// Tabs: API-Zugänge | Workflows | Hotkeys | Korrekturen | Erweitert (mit Sicherung)
"use strict";

const { invoke } = window.__TAURI__.core;
const el = (id) => document.getElementById(id);

let META = null;
let S = null; // aktuelle Einstellungen (werden beim Speichern geändert)

// ── Tabs ────────────────────────────────────────────────────────────────────

for (const btn of el("tabbar").querySelectorAll("button")) {
  btn.addEventListener("click", () => {
    for (const b of el("tabbar").querySelectorAll("button")) b.classList.toggle("active", b === btn);
    for (const p of document.querySelectorAll(".pane")) p.classList.toggle("active", p.id === "tab-" + btn.dataset.tab);
  });
}

// ── API-Zugänge ─────────────────────────────────────────────────────────────

const keyFields = {}; // Key-Name -> Eingabefeld

function keyRow(title, keyName, placeholder, stored) {
  const display = h("div", { class: "hint", text: stored ? `Gespeichert: ${stored}` : "Nicht gesetzt" });
  const field = h("input", { type: "password", placeholder });
  keyFields[keyName] = field;
  let hasStored = !!stored;

  const showBtn = h("button", { text: "Anzeigen", title: "Key vollständig in einem eigenen Fenster anzeigen" });
  const updateBtn = () => (showBtn.disabled = !(field.value.trim() || hasStored));
  field.addEventListener("input", updateBtn);
  updateBtn();
  showBtn.addEventListener("click", async () => {
    // eingetippten Key, sonst den gespeicherten Key anzeigen
    const key = field.value.trim() || (await invoke("key_get", { name: keyName })) || "";
    if (key) showKey(title, key);
  });

  const pasteBtn = h("button", { text: "Einfügen" });
  pasteBtn.addEventListener("click", async () => {
    const text = await invoke("clipboard_read");
    if (text) {
      field.value = text;
      updateBtn();
    }
  });

  const clearBtn = h("button", { text: "Löschen" });
  clearBtn.addEventListener("click", async () => {
    await invoke("key_clear", { name: keyName });
    hasStored = false;
    display.textContent = "Nicht gesetzt";
    field.value = "";
    updateBtn();
  });

  return h("section", { class: "card" }, h("h3", { text: title }), display, h("div", { class: "keyrow" }, field, showBtn, pasteBtn, clearBtn));
}

// ── OpenRouter-Modell ───────────────────────────────────────────────────────

const orCombo = editableCombo();
el("or-model-slot").replaceWith(orCombo.wrap);
const orModel = orCombo.input;
const llmModelCombos = []; // { combo, provider }
const sttModelFields = []; // { field, provider }
let orSttModels = [];

function refreshOpenrouterWorkflowCombos() {
  for (const { combo, provider } of llmModelCombos) {
    if (provider.value === "openrouter") fillLlmModels(combo, "openrouter", "");
  }
}
orModel.addEventListener("input", refreshOpenrouterWorkflowCombos);

// ── Workflows ───────────────────────────────────────────────────────────────

function languageCombo(current) {
  const { input, wrap } = editableCombo();
  input.setItems(META.languages.map(([name]) => name));
  const found = META.languages.find(([, code]) => code === (current || ""));
  input.value = found ? found[0] : current; // eigener Code, z. B. "sv"
  return { input, wrap };
}

/// Sprachcode aus der Auswahl: Listeneintrag -> Code, sonst eingetippter Text.
function languageValue(input) {
  const text = input.value.trim();
  const found = META.languages.find(([name]) => name.toLowerCase() === text.toLowerCase());
  return found ? found[1] : text;
}

/// STT-Modellfeld: bei „Lokal“ die Modellgröße, bei AssemblyAI fest, sonst
/// editierbar mit Vorschlägen.
function sttModelField(providerSelect, currentModel, localSize) {
  const holder = h("span", { style: "display: contents" });
  const f = { holder, mode: "", localSize: localSize || "base", input: null, list: null, select: null };
  f.fill = (provider, keepText) => {
    holder.replaceChildren();
    if (provider === "local_whisper") {
      f.mode = "local";
      f.select = select(META.localSizes.map(([label, size]) => [size, label]), f.localSize);
      if (f.select.value !== f.localSize) f.select.selectedIndex = 1;
      holder.append(f.select);
    } else if (provider === "assemblyai") {
      // BlitzVox schickt kein Modell mit: AssemblyAI nimmt sein Standardmodell
      f.mode = "assembly";
      holder.append(h("input", { type: "text", value: "Standardmodell von AssemblyAI", disabled: "" }));
    } else {
      f.mode = "free";
      const { input, wrap } = editableCombo();
      let items = [];
      if (provider === "openrouter_whisper") {
        items = orSttModels;
        input.placeholder = "Standard: openai/whisper-1";
      } else if (provider === "openai_whisper") {
        items = ["whisper-1"];
      } else if (provider === "groq_whisper") {
        items = ["whisper-large-v3-turbo"];
      } else if (provider === "custom") {
        input.placeholder = "Standard: whisper-1";
      }
      input.setItems(items);
      input.value = keepText ? keepText : items.length ? items[0] : "";
      f.input = input;
      holder.append(wrap);
    }
  };
  f.store = (cfg, provider) => {
    if (provider === "local_whisper") cfg.local_model_size = f.select.value || "base";
    else if (provider === "assemblyai") cfg.stt_model = "";
    else cfg.stt_model = f.input.value.trim();
  };
  f.fill(providerSelect.value, currentModel);
  // Bei echtem Anbieterwechsel wird das Modellfeld geleert
  providerSelect.addEventListener("change", () => f.fill(providerSelect.value, ""));
  sttModelFields.push({ field: f, provider: providerSelect });
  return f;
}

function fillLlmModels(combo, provider, keepText) {
  if (provider === "openrouter") {
    // Modell kommt aus dem Block „OpenRouter – Modell“ (API-Zugänge)
    combo.input.setItems([]);
    combo.input.value = orModel.value.trim();
    combo.input.disabled = true;
    combo.input.title = "Wird im Tab API-Zugänge unter 'OpenRouter – Modell' gewählt.";
  } else {
    combo.input.disabled = false;
    combo.input.title = "";
    combo.input.setItems(META.llmPresets[provider] || []);
    combo.input.value = keepText;
  }
}

function llmModelCombo(providerSelect, current) {
  const combo = editableCombo();
  fillLlmModels(combo, providerSelect.value, current);
  providerSelect.addEventListener("change", () => fillLlmModels(combo, providerSelect.value, ""));
  llmModelCombos.push({ combo, provider: providerSelect });
  return combo;
}

const W = {}; // Felder der Workflow-Gruppen

function buildWorkflows() {
  const pane = el("tab-workflows");
  const stt = (cur) => select(META.sttProviders, cur);
  const llm = (cur) => select(META.llmProviders, cur);
  const row = (form, label, field) => form.append(h("span", { text: label }), field);

  // ─ Transkription ─
  {
    const m = S.transcription_model;
    const form = h("div", { class: "form" });
    W.trStt = stt(m.stt_provider);
    W.trModel = sttModelField(W.trStt, m.stt_model, m.local_model_size);
    W.trLang = languageCombo(m.language);
    row(form, "STT-Anbieter:", W.trStt);
    row(form, "STT-Modell:", W.trModel.holder);
    row(form, "Sprache:", W.trLang.wrap);
    form.append(h("span"), h("span", { class: "hint", text: "Gilt für alle vier Workflows. Andere Sprachen als Code eintragen, z. B. „sv“ für Schwedisch." }));
    pane.append(h("section", { class: "card" }, h("h3", { text: "🎙  BlitzVox — Transkription" }), form));
  }
  // ─ BlitzVox+ ─
  {
    const ti = S.text_improvement;
    const form = h("div", { class: "form" });
    W.tiStt = stt(ti.model.stt_provider);
    W.tiModel = sttModelField(W.tiStt, ti.model.stt_model, ti.model.local_model_size);
    W.tiLlm = llm(ti.model.llm_provider);
    W.tiLlmModel = llmModelCombo(W.tiLlm, ti.model.llm_model);
    W.tiTone = select([["formal", "Formell"], ["neutral", "Neutral"], ["casual", "Locker"]], ti.tone);
    W.tiContext = h("input", { type: "text" });
    W.tiContext.value = ti.context;
    W.tiPrompt = h("textarea", { class: "prompt", style: "height: 80px", placeholder: "Leer = automatischer Standardprompt" });
    W.tiPrompt.value = ti.system_prompt;
    row(form, "STT-Anbieter:", W.tiStt);
    row(form, "STT-Modell:", W.tiModel.holder);
    row(form, "LLM-Anbieter:", W.tiLlm);
    row(form, "LLM-Modell:", W.tiLlmModel.wrap);
    row(form, "Ton:", W.tiTone);
    row(form, "Kontext:", W.tiContext);
    row(form, "Eigener Prompt:", W.tiPrompt);
    pane.append(h("section", { class: "card" }, h("h3", { text: "✨  BlitzVox+ — Text verbessern" }), form));
  }
  // ─ Dampf ablassen ─
  {
    const da = S.dampf_ablassen;
    const form = h("div", { class: "form" });
    W.daStt = stt(da.model.stt_provider);
    W.daModel = sttModelField(W.daStt, da.model.stt_model, da.model.local_model_size);
    W.daLlm = llm(da.model.llm_provider);
    W.daLlmModel = llmModelCombo(W.daLlm, da.model.llm_model);
    W.daPrompt = h("textarea", { class: "prompt", style: "height: 100px" });
    W.daPrompt.value = da.system_prompt;
    row(form, "STT-Anbieter:", W.daStt);
    row(form, "STT-Modell:", W.daModel.holder);
    row(form, "LLM-Anbieter:", W.daLlm);
    row(form, "LLM-Modell:", W.daLlmModel.wrap);
    row(form, "System-Prompt:", W.daPrompt);
    pane.append(h("section", { class: "card" }, h("h3", { text: "🔥  BlitzVox $%&! — Frust umformulieren" }), form));
  }
  // ─ Emoji ─
  {
    const em = S.emoji;
    const form = h("div", { class: "form" });
    W.emStt = stt(em.model.stt_provider);
    W.emModel = sttModelField(W.emStt, em.model.stt_model, em.model.local_model_size);
    W.emLlm = llm(em.model.llm_provider);
    W.emLlmModel = llmModelCombo(W.emLlm, em.model.llm_model);
    W.emDensity = select([["wenig", "Wenig"], ["mittel", "Mittel"], ["viel", "Viel"]], em.density);
    row(form, "STT-Anbieter:", W.emStt);
    row(form, "STT-Modell:", W.emModel.holder);
    row(form, "LLM-Anbieter:", W.emLlm);
    row(form, "LLM-Modell:", W.emLlmModel.wrap);
    row(form, "Emoji-Dichte:", W.emDensity);
    pane.append(h("section", { class: "card" }, h("h3", { text: "😊  BlitzVox :) — Emojis" }), form));
  }
}

// ── Hotkeys ─────────────────────────────────────────────────────────────────

const HK = {};

function modeCombo(current) {
  return select([["hold", "Halten"], ["toggle", "Umschalten"]], current === "hold" ? "hold" : "toggle");
}

function buildHotkeys() {
  const hk = S.hotkeys;
  el("hk-enabled").checked = hk.enabled;
  const grid = el("hk-grid");
  HK.transcription = hotkeyEdit(hk.transcription);
  HK.text_improvement = hotkeyEdit(hk.text_improvement);
  HK.dampf_ablassen = hotkeyEdit(hk.dampf_ablassen);
  HK.emoji_text = hotkeyEdit(hk.emoji_text);
  HK.correction = hotkeyEdit(hk.correction);
  HK.transcription_mode = modeCombo(hk.transcription_mode);
  HK.text_improvement_mode = modeCombo(hk.text_improvement_mode);
  HK.dampf_ablassen_mode = modeCombo(hk.dampf_ablassen_mode);
  HK.emoji_text_mode = modeCombo(hk.emoji_text_mode);

  const rows = [
    ["🎙  BlitzVox", "kurze Texte", HK.transcription, HK.transcription_mode],
    ["✨  BlitzVox+", "lange Texte / freies Sprechen", HK.text_improvement, HK.text_improvement_mode],
    ["🔥  BlitzVox $%&!", "Frust umformulieren", HK.dampf_ablassen, HK.dampf_ablassen_mode],
    ["😊  BlitzVox :)", "mit Emojis", HK.emoji_text, HK.emoji_text_mode],
    ["✏  Korrigieren", "Fenster für letzten Text öffnen", HK.correction, null],
  ];
  for (const [name, desc, field, mode] of rows) {
    grid.append(h("span", { text: name, title: desc, style: "white-space: pre" }), field, mode || h("span", { class: "hint", text: "—" }));
  }
  const help = h("div", { class: "hint span" });
  help.innerHTML =
    "<b>Hotkeys ändern:</b> Ins Feld klicken und die gewünschte Kombination drücken, " +
    "z.&nbsp;B. Strg+Alt+W (wird als ctrl+alt+w angezeigt). " +
    "Entf oder Rücktaste leert das Feld.<br>" +
    "<b>Halten:</b> Kombination gedrückt halten, solange du sprichst. " +
    "Beim Loslassen endet die Aufnahme und der Text wird verarbeitet.<br>" +
    "<b>Umschalten:</b> Einmal drücken startet die Aufnahme, nochmal drücken stoppt sie.<br>" +
    "Änderungen gelten nach „Speichern“.";
  grid.append(help);

  const reset = h("button", {
    class: "span",
    text: "Standardwerte wiederherstellen",
    title: "Setzt alle Tastenkürzel und Modi auf die Vorgaben zurück. Übernommen wird erst mit „Speichern“.",
    style: "justify-self: start",
  });
  reset.addEventListener("click", () => {
    const d = META.defaults.hotkeys;
    for (const k of ["transcription", "text_improvement", "dampf_ablassen", "emoji_text", "correction"]) HK[k].value = d[k];
    for (const k of ["transcription_mode", "text_improvement_mode", "dampf_ablassen_mode", "emoji_text_mode"]) HK[k].value = d[k];
  });
  grid.append(reset);

  // Tabelle ausgrauen, wenn Hotkeys deaktiviert sind
  const sync = () => el("hk-card").classList.toggle("disabled", !el("hk-enabled").checked);
  el("hk-enabled").addEventListener("change", sync);
  sync();
}

// ── Korrekturen ─────────────────────────────────────────────────────────────

let lastClicked = -1;

async function corrReload() {
  const rows = await invoke("corr_list");
  const body = el("corr-body");
  body.replaceChildren();
  rows.forEach(([wrong, right], i) => {
    const tr = h("tr", {}, h("td", { text: wrong }), h("td", { text: right }));
    tr.dataset.wrong = wrong;
    tr.addEventListener("click", (e) => {
      const all = [...body.children];
      if (e.shiftKey && lastClicked >= 0) {
        const [a, b] = [Math.min(lastClicked, i), Math.max(lastClicked, i)];
        all.forEach((r, j) => r.classList.toggle("selected", j >= a && j <= b));
      } else if (e.ctrlKey) {
        tr.classList.toggle("selected");
        lastClicked = i;
      } else {
        all.forEach((r) => r.classList.toggle("selected", r === tr));
        lastClicked = i;
      }
    });
    body.append(tr);
  });
  lastClicked = -1;
}

el("corr-add").addEventListener("click", async () => {
  const wrong = el("corr-wrong").value;
  const right = el("corr-right").value;
  if (await invoke("corr_add", { wrong, right })) {
    el("corr-wrong").value = "";
    el("corr-right").value = "";
    await corrReload();
  }
});

el("corr-delete").addEventListener("click", async () => {
  const wrongs = [...el("corr-body").querySelectorAll("tr.selected")].map((r) => r.dataset.wrong);
  await invoke("corr_remove", { wrongs });
  await corrReload();
});

// ── Erweitert ───────────────────────────────────────────────────────────────

async function buildAdvanced() {
  el("auto-paste").checked = S.auto_paste;
  // 2.0 braucht unter Windows nichts zusätzlich (1.0.8 nannte hier pyautogui)
  el("paste-note").textContent =
    "Auto-Paste benötigt xdotool (Linux X11) oder ydotool (Wayland).\n" +
    "Ohne Auto-Paste landet der Text nur in der Zwischenablage.";
  el("launch-label").textContent = META.windows ? "Mit Windows starten" : "Beim Anmelden starten";
  el("launch").checked = S.launch_at_login;
  el("version").textContent = `Version ${META.version}`;

  const mic = el("mic");
  try {
    const devices = await invoke("input_devices");
    for (const d of devices) {
      mic.append(h("option", { value: d.id, text: d.name + (d.default ? "  (Default)" : "") }));
    }
    mic.value = S.audio_input_device_id || "";
    if (mic.selectedIndex < 0) mic.selectedIndex = 0;
  } catch (e) {
    mic.append(h("option", { value: "", text: `Fehler: ${e}` }));
  }
}

el("wizard").addEventListener("click", () => invoke("request_setup_wizard"));

// ── Sicherung ───────────────────────────────────────────────────────────────

const MIN_BACKUP_PW = 12;

/// Knöpfe sperren, solange gearbeitet wird (Argon2 braucht einen Moment).
function backupBusy(busy) {
  for (const id of ["bk-create", "bk-restore", "save"]) el(id).disabled = busy;
}

el("bk-create").addEventListener("click", async () => {
  const status = el("bk-create-status");
  const pw = el("bk-pw1").value;
  if ([...pw].length < MIN_BACKUP_PW) {
    status.textContent = `Das Passwort muss mindestens ${MIN_BACKUP_PW} Zeichen lang sein.`;
    return;
  }
  if (pw !== el("bk-pw2").value) {
    status.textContent = "Die beiden Passwörter stimmen nicht überein.";
    return;
  }
  backupBusy(true);
  status.textContent = "Sicherung wird erstellt …";
  try {
    const path = await invoke("backup_create", { password: pw });
    if (path) {
      status.textContent = "Gespeichert: " + path;
      el("bk-pw1").value = "";
      el("bk-pw2").value = "";
    } else status.textContent = "Abgebrochen.";
  } catch (e) {
    status.textContent = String(e);
  } finally {
    backupBusy(false);
  }
});

el("bk-restore").addEventListener("click", async () => {
  const status = el("bk-restore-status");
  const pw = el("bk-pw-restore").value;
  if (!pw) {
    status.textContent = "Bitte das Passwort der Sicherung eingeben.";
    return;
  }
  backupBusy(true);
  status.textContent = "Sicherung wird geprüft …";
  try {
    if (await invoke("backup_restore", { password: pw })) {
      el("bk-pw-restore").value = "";
      location.reload(); // Fenster mit den eingespielten Werten neu aufbauen
      return;
    }
    status.textContent = "Abgebrochen, nichts geändert.";
  } catch (e) {
    status.textContent = String(e);
  } finally {
    backupBusy(false);
  }
});

// ── Speichern ───────────────────────────────────────────────────────────────

async function save() {
  const s = S;
  const keys = {};
  for (const [name, field] of Object.entries(keyFields)) keys[name] = field.value.trim();

  s.ollama_base_url = el("ollama-url").value.trim();
  s.ollama_llm_model = el("ollama-model").value.trim();
  s.custom_stt_url = el("custom-stt-url").value.trim();
  s.custom_llm_url = el("custom-llm-url").value.trim();
  s.openrouter_llm_model = orModel.value.trim();

  s.transcription_model.stt_provider = W.trStt.value;
  W.trModel.store(s.transcription_model, W.trStt.value);
  // Sprache gilt für alle vier Workflows (wie im Einrichtungsassistenten)
  const language = languageValue(W.trLang.input);
  for (const m of [s.transcription_model, s.text_improvement.model, s.dampf_ablassen.model, s.emoji.model]) m.language = language;

  s.text_improvement.model.stt_provider = W.tiStt.value;
  W.tiModel.store(s.text_improvement.model, W.tiStt.value);
  s.text_improvement.model.llm_provider = W.tiLlm.value;
  s.text_improvement.model.llm_model = W.tiLlmModel.input.value.trim();
  s.text_improvement.tone = W.tiTone.value;
  s.text_improvement.context = W.tiContext.value.trim();
  s.text_improvement.system_prompt = W.tiPrompt.value.trim();

  s.dampf_ablassen.model.stt_provider = W.daStt.value;
  W.daModel.store(s.dampf_ablassen.model, W.daStt.value);
  s.dampf_ablassen.model.llm_provider = W.daLlm.value;
  s.dampf_ablassen.model.llm_model = W.daLlmModel.input.value.trim();
  s.dampf_ablassen.system_prompt = W.daPrompt.value.trim();

  s.emoji.model.stt_provider = W.emStt.value;
  W.emModel.store(s.emoji.model, W.emStt.value);
  s.emoji.model.llm_provider = W.emLlm.value;
  s.emoji.model.llm_model = W.emLlmModel.input.value.trim();
  s.emoji.density = W.emDensity.value;

  const hk = s.hotkeys;
  for (const k of ["transcription", "text_improvement", "dampf_ablassen", "emoji_text", "correction"]) hk[k] = HK[k].value.trim();
  for (const k of ["transcription_mode", "text_improvement_mode", "dampf_ablassen_mode", "emoji_text_mode"]) hk[k] = HK[k].value;
  hk.enabled = el("hk-enabled").checked;

  s.auto_paste = el("auto-paste").checked;
  s.spoken_punctuation = el("spoken-punct").checked;
  s.launch_at_login = el("launch").checked;
  s.audio_input_device_id = el("mic").value || null;

  await invoke("settings_save", { settings: s, keys });
  await invoke("close_window");
}

el("save").addEventListener("click", () => save().catch((e) => console.error(e)));
el("cancel").addEventListener("click", () => invoke("close_window"));

// ── Start ───────────────────────────────────────────────────────────────────

async function init() {
  META = await invoke("settings_meta");
  S = await invoke("settings_get");
  const stored = await invoke("keys_status");

  el("keys-top").replaceWith(
    keyRow("Groq API Key (kostenlos, Standard)", "groq_api_key", "gsk_…", stored.groq_api_key),
    keyRow("Claude API Key (Anthropic)", "anthropic_api_key", "sk-ant-…", stored.anthropic_api_key),
    keyRow("OpenAI API Key", "openai_api_key", "sk-…", stored.openai_api_key),
    keyRow("AssemblyAI API Key", "assemblyai_api_key", "…", stored.assemblyai_api_key),
  );
  el("keys-or").replaceWith(keyRow("OpenRouter API Key", "openrouter_api_key", "sk-or-v1-…", stored.openrouter_api_key));

  el("ollama-url").value = S.ollama_base_url;
  el("ollama-model").value = S.ollama_llm_model;
  el("custom-stt-url").value = S.custom_stt_url;
  el("custom-llm-url").value = S.custom_llm_url;
  for (const k of ["custom_stt_api_key", "custom_llm_api_key"]) {
    const f = el("key-" + k);
    f.placeholder = stored[k] ? `Gespeichert: ${stored[k]}` : "Nicht gesetzt";
    keyFields[k] = f;
  }
  orModel.value = S.openrouter_llm_model;

  buildWorkflows();
  buildHotkeys();
  el("spoken-punct").checked = S.spoken_punctuation;
  corrReload();
  buildAdvanced();

  // OpenRouter-Modelllisten beim Öffnen laden
  invoke("openrouter_models").then((ids) => {
    const current = orModel.value;
    orModel.setItems(ids);
    orModel.value = current;
    el("or-status").textContent = ids.length
      ? `${ids.length} Modelle geladen. Auswahl oder eigene ID eintragen.`
      : "Keine Modellliste (offline oder kein Key). Modell-ID von Hand eintragen.";
  });
  invoke("openrouter_stt_models").then((ids) => {
    orSttModels = ids;
    for (const { field, provider } of sttModelFields) {
      if (provider.value === "openrouter_whisper") field.fill("openrouter_whisper", field.input ? field.input.value : "");
    }
  });
}

init().catch((e) => console.error(e));
