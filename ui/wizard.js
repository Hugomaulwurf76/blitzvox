// Einrichtungsassistent – Nachbau von ui/setup_wizard.py (Version 1.0.8).
//
// Seiten:
//   1. Willkommen + Diktiersprache
//   2. Spracherkennung + API-Key (mit Key-Test)
//   3. KI-Funktionen (optional)
//   4. Mikrofon (mit Pegelanzeige)
//   5. Tastenkürzel
//   6. Probediktat (echte Aufnahme + Transkription mit den gewählten Werten)
//   7. Abschluss (Automatisch einfügen, Autostart, Zusammenfassung)
//
// Der Assistent arbeitet auf einer Kopie der Einstellungen und gibt sie erst
// bei „Fertig“ zurück. Ausnahme: API-Keys werden beim Testen bzw. Weiterklicken
// im Schlüsselbund gespeichert, weil das Probediktat sie braucht.
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let S = null; // Kopie der Einstellungen
let META = null;
let INFO = null; // Systeminfos (Wayland, xdotool, Grafikkarten)

// ── Feste Daten ─────────────────────────────────────────────────────────────

/// STT-Anbieter, die zwingend einen API-Key brauchen.
const STT_KEY_REQUIRED = {
  groq_whisper: "groq_api_key",
  openai_whisper: "openai_api_key",
  assemblyai: "assemblyai_api_key",
  openrouter_whisper: "openrouter_api_key",
};
/// LLM-Anbieter, die einen API-Key brauchen.
const LLM_KEY_REQUIRED = {
  groq: "groq_api_key",
  openai: "openai_api_key",
  anthropic: "anthropic_api_key",
  openrouter: "openrouter_api_key",
};
/// Wo man den Key bekommt.
const KEY_LINKS = {
  groq_api_key: ["Groq (kostenlos)", "https://console.groq.com"],
  openai_api_key: ["OpenAI", "https://platform.openai.com"],
  assemblyai_api_key: ["AssemblyAI", "https://www.assemblyai.com"],
  openrouter_api_key: ["OpenRouter", "https://openrouter.ai"],
  anthropic_api_key: ["Anthropic", "https://console.anthropic.com"],
};
const KEY_PLACEHOLDERS = {
  groq_api_key: "gsk_…",
  openai_api_key: "sk-…",
  assemblyai_api_key: "…",
  openrouter_api_key: "sk-or-v1-…",
  anthropic_api_key: "sk-ant-…",
};
/// Standard-STT-Modell je Anbieter (leer = Vorgabe der Spracherkennung).
const STT_DEFAULT_MODEL = { groq_whisper: "whisper-large-v3-turbo", openai_whisper: "whisper-1" };

const KEY_LABELS = { ctrl: "Strg", alt: "Alt", shift: "Umschalt", cmd: "Win", meta: "Win", space: "Leertaste" };

/// 'ctrl+alt+w' -> 'Strg+Alt+W'
function formatHotkey(combo) {
  const parts = [];
  for (let p of (combo || "").split("+")) {
    p = p.trim().toLowerCase();
    if (!p) continue;
    parts.push(KEY_LABELS[p] || (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1)));
  }
  return parts.join("+") || "—";
}

function workflowModels(s) {
  return [s.transcription_model, s.text_improvement.model, s.dampf_ablassen.model, s.emoji.model];
}
function llmModels(s) {
  return [s.text_improvement.model, s.dampf_ablassen.model, s.emoji.model];
}
function displayName(list, id) {
  const f = list.find(([v]) => v === id);
  return f ? f[1] : null;
}

// ── Kleine Bausteine ────────────────────────────────────────────────────────

function hint(text) {
  return h("div", { class: "hint-line", text });
}

/// Statuszeile setzen: role "hint" | "success" | "error" | "status"; leer = aus.
function setStatus(elem, role, text) {
  elem.className = { hint: "hint-line", success: "success-line", error: "error-text", status: "status-line" }[role] || "hint-line";
  elem.textContent = text;
  elem.classList.toggle("hidden", !text);
}

function clearWizardError() {
  document.getElementById("error").classList.add("hidden");
}

function overviewLabel() {
  return h("div", { class: "overview" });
}
function setOverview(label, names) {
  label.textContent = names.length ? "Eingerichtet: " + names.join(", ") : "Eingerichtet: noch keiner";
}

function infoBox(title, text) {
  return h("div", { class: "infobox" }, h("div", { class: "strong", text: title }), h("div", { text }));
}

/// Link, der im Standardbrowser geöffnet wird.
function link(url) {
  const a = h("a", { href: url, text: url });
  a.addEventListener("click", (e) => {
    e.preventDefault();
    invoke("open_url", { url });
  });
  return a;
}

function formRow(label, field) {
  return h("div", { class: "form" }, h("span", { text: label }), field);
}

// ── Baustein: API-Key-Feld mit Test ─────────────────────────────────────────

/// Eingabe für einen API-Key: Feld, Anzeigen, Einfügen, Testen, Status.
/// Ein erfolgreich getesteter Key wird sofort gespeichert.
class KeyBox {
  constructor(onSaved) {
    this.onSaved = onSaved;
    this.keyName = null;
    this.verified = null; // [keyName, key] nach „nicht erreichbar“
    this.token = 0;
    this.hasStored = false;

    this.linkLine = h("div");
    this.stored = hint("");
    this.field = h("input", { type: "password" });
    this.field.addEventListener("input", () => this.onTextChanged());
    this.showBtn = h("button", { text: "Anzeigen", title: "Key vollständig in einem eigenen Fenster anzeigen" });
    this.showBtn.addEventListener("click", () => this.showKey());
    const paste = h("button", { text: "Einfügen" });
    paste.addEventListener("click", async () => {
      const text = await invoke("clipboard_read");
      if (text) {
        this.field.value = text;
        this.onTextChanged();
      }
    });
    this.testBtn = h("button", { text: "Key testen" });
    this.testBtn.addEventListener("click", () => this.test());
    this.status = h("div", { class: "hidden" });
    this.root = h("div", { style: "display: flex; flex-direction: column; gap: 6px" },
      this.linkLine, this.stored, h("div", { class: "keyrow" }, this.field, this.showBtn, paste, this.testBtn), this.status);
  }

  async setKeyName(keyName) {
    if (keyName === this.keyName) return;
    this.keyName = keyName;
    this.verified = null;
    this.token++;
    this.field.value = "";
    const [name, url] = KEY_LINKS[keyName] || ["", ""];
    this.linkLine.replaceChildren(`Key erstellen bei ${name}: `, link(url));
    this.field.placeholder = KEY_PLACEHOLDERS[keyName] || "";
    await this.showStored();
    setStatus(this.status, "hint", "");
    this.testBtn.disabled = false;
  }

  async showStored() {
    const masked = await invoke("key_masked", { name: this.keyName });
    this.hasStored = !!masked;
    this.stored.textContent = masked
      ? `Gespeichert: ${masked}. Feld leer lassen, um diesen Key zu behalten.`
      : "Noch kein Key gespeichert.";
    this.updateShowBtn();
  }

  updateShowBtn() {
    this.showBtn.disabled = !(this.field.value.trim() || this.hasStored);
  }

  async showKey() {
    const key = this.field.value.trim() || (await invoke("key_get", { name: this.keyName })) || "";
    if (!key) return;
    const name = (KEY_LINKS[this.keyName] || [""])[0].split(" (")[0];
    showKey(`${name} API-Key`, key);
  }

  onTextChanged() {
    this.verified = null;
    setStatus(this.status, "hint", "");
    this.updateShowBtn();
  }

  async test() {
    const key = this.field.value.trim() || (this.hasStored ? (await invoke("key_get", { name: this.keyName })) || "" : "");
    if (!key) {
      setStatus(this.status, "error", "Bitte zuerst einen Key eintragen.");
      return;
    }
    const token = ++this.token;
    const keyName = this.keyName;
    this.testBtn.disabled = true;
    setStatus(this.status, "hint", "Key wird geprüft …");
    const [status, message] = await invoke("key_check", { name: keyName, key });
    if (token !== this.token) return; // veraltetes Ergebnis
    this.testBtn.disabled = false;
    if (status === "ok") {
      const [ok, error] = await this.save(keyName, key);
      if (!ok) {
        setStatus(this.status, "error", error);
        return;
      }
      setStatus(this.status, "success", "✓ " + message + " Gespeichert.");
      clearWizardError();
    } else if (status === "unreachable") {
      // Key lässt sich nicht prüfen (z. B. offline) -> trotzdem zulassen
      this.verified = [keyName, key];
      setStatus(this.status, "hint", message + " Der Key wird ungeprüft übernommen.");
    } else {
      setStatus(this.status, "error", message);
    }
  }

  /// Key speichern, Feld leeren. Liefert [ok, Fehlermeldung].
  async save(keyName, key) {
    try {
      await invoke("key_store", { name: keyName, key });
    } catch (e) {
      return [false, `Der Key konnte nicht im Schlüsselbund des Systems gespeichert werden: ${e}`];
    }
    this.field.value = "";
    this.verified = null;
    await this.showStored();
    if (this.onSaved) this.onSaved(keyName);
    return [true, ""];
  }

  /// Für „Weiter“: Key muss getestet sein. Liefert [ok, Fehlermeldung].
  async commit() {
    const typed = this.field.value.trim();
    if (typed) {
      if (!this.verified || this.verified[0] !== this.keyName || this.verified[1] !== typed) {
        return [false, "Bitte den Key zuerst mit „Key testen“ prüfen."];
      }
      // nur bei „Anbieter nicht erreichbar“: ungeprüft übernehmen
      return this.save(this.keyName, typed);
    }
    if (await invoke("key_masked", { name: this.keyName })) return [true, ""];
    return [false, "Bitte einen API-Key eintragen und mit „Key testen“ prüfen."];
  }
}

// ── Seiten ──────────────────────────────────────────────────────────────────

class Page {
  constructor(title, lead) {
    this.body = h("div", { class: "body" });
    this.root = h("div", { class: "page" }, h("div", { class: "display-sm", text: title }), h("div", { class: "lead", text: lead }), this.body);
  }
  async load() {}
  async enter() {}
  leave() {}
  store() {}
  async canContinue() {
    return [true, ""];
  }
}

// 1 ── Willkommen

class WelcomePage extends Page {
  constructor() {
    super("Willkommen bei BlitzVox",
      "BlitzVox wandelt Gesprochenes per Tastenkürzel in Text um und fügt ihn dort ein, wo dein Cursor steht. In wenigen Schritten richten wir alles ein.");
    this.language = h("select");
    this.body.append(
      h("div", { class: "form" }, h("span", { text: "In welcher Sprache diktierst du?" }), this.language),
      hint("Eine feste Sprache liefert die genauesten Ergebnisse. „Automatisch erkennen“ ist sinnvoll, wenn du oft die Sprache wechselst. Weitere Sprachen kannst du später in den Einstellungen unter „Workflows“ eintragen."),
      h("div", { style: "height: 8px" }),
      infoBox("Was passiert, wenn du abbrichst?",
        "Deine Einstellungen werden erst mit „Fertig“ im letzten Schritt übernommen. Brichst du vorher ab, bleiben nur die API-Keys gespeichert, die du bereits getestet oder mit „Weiter“ bestätigt hast."),
    );
  }
  async load() {
    for (const [name, code] of META.languages) this.language.append(h("option", { value: code, text: name }));
    const code = S.transcription_model.language || "";
    if (!META.languages.some(([, c]) => c === code)) {
      // Sprache wurde vorher in den Einstellungen von Hand eingetragen
      this.language.append(h("option", { value: code, text: `Eigene Einstellung (${code})` }));
    }
    this.language.value = code;
  }
  store() {
    const code = this.language.value || "";
    for (const m of workflowModels(S)) m.language = code;
  }
}

// 2 ── Spracherkennung

class SpeechPage extends Page {
  constructor() {
    super("Spracherkennung",
      "Wähle, welcher Dienst deine Sprache in Text umwandelt. Groq ist kostenlos und für den Start empfohlen.");
    this.initialProvider = null;
    this.overview = overviewLabel();
    this.provider = h("select");
    this.provider.addEventListener("change", () => this.updateMode());
    this.keyBox = new KeyBox(() => this.updateOverview());

    // Bereich: lokal (in 2.0 fest eingebaut, whisper.cpp)
    this.localSize = h("select");
    this.localGpu = hint("");
    this.localBox = h("div", { style: "display: flex; flex-direction: column; gap: 10px" },
      formRow("Modellgröße:", this.localSize),
      hint("Läuft komplett auf deinem Rechner, ohne Internet und ohne Key. Beim ersten Diktat wird das gewählte Modell einmalig heruntergeladen."),
      this.localGpu);

    // Bereich: eigener Endpunkt
    this.customUrl = h("input", { type: "text", placeholder: "https://…/v1/audio/transcriptions" });
    this.customKey = h("input", { type: "password" });
    this.customBox = h("div", { class: "form" },
      h("span", { text: "Adresse (URL):" }), this.customUrl,
      h("span", { text: "Key (optional):" }), this.customKey);

    this.body.append(this.overview, formRow("Dienst:", this.provider), this.keyBox.root, this.localBox, this.customBox,
      hint("Du kannst mehrere Dienste nacheinander einrichten: Dienst wählen, Key eintragen, „Key testen“ – dann den nächsten Dienst wählen. Jeder getestete Key wird sofort gespeichert. BlitzVox nutzt den Dienst, der beim Klick auf „Weiter“ ausgewählt ist. Die anderen kannst du später in den Einstellungen unter „Workflows“ jedem Workflow einzeln zuweisen."),
      hint("API-Keys werden sicher im Schlüsselbund deines Systems gespeichert, nicht in einer Datei."));
  }

  async updateOverview() {
    const names = [];
    for (const [p, key] of Object.entries(STT_KEY_REQUIRED)) {
      if (await invoke("key_masked", { name: key })) names.push(displayName(META.sttProviders, p).split(" (")[0]);
    }
    setOverview(this.overview, names);
  }

  async updateMode() {
    this.updateOverview();
    const p = this.provider.value;
    const needsKey = p in STT_KEY_REQUIRED;
    this.keyBox.root.classList.toggle("hidden", !needsKey);
    if (needsKey) await this.keyBox.setKeyName(STT_KEY_REQUIRED[p]);
    this.localBox.classList.toggle("hidden", p !== "local_whisper");
    // Neuer Text für 2.0: whisper.cpp nutzt die Grafikkarte über Vulkan
    this.localGpu.textContent = INFO.gpus.length
      ? `Grafikkarte erkannt (${INFO.gpus[0]}): BlitzVox nutzt sie für die Erkennung. Klappt das nicht, rechnet automatisch der Prozessor.`
      : "Keine passende Grafikkarte erkannt: Die Erkennung läuft auf dem Prozessor. Das belastet ihn und ist langsamer als die Online-Dienste.";
    this.customBox.classList.toggle("hidden", p !== "custom");
  }

  async load() {
    for (const [id, name] of META.sttProviders) this.provider.append(h("option", { value: id, text: name }));
    for (const [name, size] of META.localSizes) this.localSize.append(h("option", { value: size, text: name }));
    const cfg = S.transcription_model;
    this.initialProvider = cfg.stt_provider;
    this.provider.value = cfg.stt_provider;
    if (this.provider.selectedIndex < 0) this.provider.selectedIndex = 0;
    this.localSize.value = cfg.local_model_size || "base";
    if (this.localSize.selectedIndex < 0) this.localSize.selectedIndex = 1;
    this.customUrl.value = cfg.custom_stt_url || S.custom_stt_url;
    const stored = await invoke("key_masked", { name: "custom_stt_api_key" });
    this.customKey.placeholder = stored ? `Gespeichert: ${stored}` : "Nicht gesetzt";
    await this.updateMode();
  }

  async canContinue() {
    const p = this.provider.value;
    if (p in STT_KEY_REQUIRED) return this.keyBox.commit();
    if (p === "custom") {
      const url = this.customUrl.value.trim();
      if (!(url.startsWith("http://") || url.startsWith("https://"))) {
        return [false, "Bitte eine gültige Adresse eintragen (beginnt mit http:// oder https://)."];
      }
      const key = this.customKey.value.trim();
      if (key) {
        try {
          await invoke("key_store", { name: "custom_stt_api_key", key });
        } catch (e) {
          return [false, `Der Key konnte nicht gespeichert werden: ${e}`];
        }
      }
    }
    return [true, ""];
  }

  store() {
    const p = this.provider.value;
    const url = this.customUrl.value.trim();
    for (const m of workflowModels(S)) {
      // Anderer Dienst -> dessen Standardmodell, sonst eigenes Modell behalten
      if (p !== this.initialProvider) m.stt_model = STT_DEFAULT_MODEL[p] || "";
      m.stt_provider = p;
      m.local_model_size = this.localSize.value;
      if (p === "custom") m.custom_stt_url = url;
    }
    if (p === "custom") S.custom_stt_url = url;
  }
}

// 3 ── KI-Funktionen

class AiPage extends Page {
  constructor() {
    super("KI-Funktionen",
      "Für „Text verbessern“, „Frust umformulieren“ und „Emojis“ braucht BlitzVox ein Sprachmodell. Dieser Schritt ist optional.");
    this.token = 0;
    this.orToken = 0;
    this.ollamaOk = false;
    this.sttProvider = null;
    this.wantedModel = "";

    this.enabled = h("input", { type: "checkbox" });
    this.enabled.checked = true;
    this.enabled.addEventListener("change", () => this.updateMode());

    this.overview = overviewLabel();
    this.provider = h("select");
    this.provider.addEventListener("change", () => this.onProviderChanged());
    const combo = editableCombo();
    this.model = combo.input;
    this.modelStatus = h("div", { class: "hidden" });
    this.sameKey = hint("Es wird derselbe Key wie bei der Spracherkennung verwendet.");
    this.keyBox = new KeyBox(() => this.updateOverview());

    this.ollamaUrl = h("input", { type: "text", placeholder: "http://localhost:11434" });
    this.ollamaUrl.addEventListener("input", () => this.resetOllama());
    this.ollamaTest = h("button", { text: "Verbindung testen" });
    this.ollamaTest.addEventListener("click", () => this.testOllama());
    this.ollamaStatus = h("div", { class: "hidden" });
    this.ollamaBox = h("div", { style: "display: flex; flex-direction: column; gap: 6px" },
      h("div", { class: "row" }, h("span", { text: "Adresse:" }), this.ollamaUrl, this.ollamaTest),
      this.ollamaStatus,
      hint("Ollama muss auf deinem Rechner installiert sein und laufen. Das Modell lädst du vorher mit „ollama pull <name>“ herunter."));

    this.box = h("div", { style: "display: flex; flex-direction: column; gap: 10px" },
      this.overview,
      h("div", { class: "form" }, h("span", { text: "Anbieter:" }), this.provider, h("span", { text: "Modell:" }), combo.wrap),
      this.modelStatus, this.sameKey, this.keyBox.root,
      hint("Auch hier kannst du mehrere Anbieter nacheinander einrichten. Genutzt wird der Anbieter, der beim Klick auf „Weiter“ ausgewählt ist."),
      this.ollamaBox);
    this.skipHint = hint("Ohne Sprachmodell funktioniert die normale Transkription weiterhin. Die KI-Funktionen kannst du später in den Einstellungen einrichten.");
    this.body.append(h("label", { class: "check" }, this.enabled, h("span", { text: "KI-Funktionen jetzt einrichten" })), this.box, this.skipHint);
  }

  /// Wie eine editierbare Qt-Auswahlliste: nach dem Füllen steht der erste Eintrag drin.
  setModels(names) {
    this.model.setItems(names);
    this.model.value = names.length ? names[0] : "";
  }

  fillModels(names) {
    this.setModels(names);
    if (this.wantedModel && names.includes(this.wantedModel)) this.model.value = this.wantedModel;
    else if (this.wantedModel && this.provider.value === "openrouter") this.model.value = this.wantedModel;
  }

  async onProviderChanged() {
    const p = this.provider.value;
    this.setModels([]);
    this.modelStatus.classList.add("hidden");
    if (p === "openrouter") {
      // Liste live von OpenRouter laden (wie im Einstellungsfenster)
      this.model.placeholder = "Modell-ID, z. B. von openrouter.ai/models";
      this.loadOpenrouterModels();
    } else if (p === "ollama") {
      // Nur Modelle zeigen, die in Ollama wirklich installiert sind
      this.model.placeholder = "Erst „Verbindung testen“";
      this.testOllama();
    } else {
      this.model.placeholder = "";
      this.setModels(META.llmPresets[p] || []);
    }
    await this.updateMode();
  }

  async loadOpenrouterModels() {
    const token = ++this.orToken;
    setStatus(this.modelStatus, "hint", "Modelle werden von OpenRouter geladen …");
    const ids = await invoke("openrouter_models");
    if (token !== this.orToken || this.provider.value !== "openrouter") return;
    this.fillModels(ids);
    if (ids.length) {
      setStatus(this.modelStatus, "hint", `${ids.length} Modelle geladen. Auswählen oder eine Modell-ID eintippen.`);
    } else {
      this.model.value = this.wantedModel;
      setStatus(this.modelStatus, "hint", "Modellliste nicht erreichbar (offline?). Modell-ID bitte von Hand eintragen.");
    }
  }

  /// Nutzt das Sprachmodell denselben, schon gespeicherten Key wie die STT?
  async keyShared(keyName) {
    return !!keyName && keyName === STT_KEY_REQUIRED[this.sttProvider] && !!(await invoke("key_masked", { name: keyName }));
  }

  async updateOverview() {
    const names = [];
    for (const [p, key] of Object.entries(LLM_KEY_REQUIRED)) {
      if (await invoke("key_masked", { name: key })) names.push(displayName(META.llmProviders, p).split(" (")[0]);
    }
    if (this.ollamaOk) names.push("Lokal – Ollama");
    setOverview(this.overview, names);
  }

  async updateMode() {
    this.updateOverview();
    const on = this.enabled.checked;
    this.box.classList.toggle("hidden", !on);
    this.skipHint.classList.toggle("hidden", on);
    const p = this.provider.value;
    const keyName = LLM_KEY_REQUIRED[p] || null;
    const shared = await this.keyShared(keyName);
    this.sameKey.classList.toggle("hidden", !shared);
    this.keyBox.root.classList.toggle("hidden", !(keyName && !shared));
    if (keyName) await this.keyBox.setKeyName(keyName);
    this.ollamaBox.classList.toggle("hidden", p !== "ollama");
  }

  resetOllama() {
    this.ollamaOk = false;
    setStatus(this.ollamaStatus, "hint", "");
    if (this.provider.value === "ollama") this.setModels([]);
  }

  async testOllama() {
    const url = this.ollamaUrl.value.trim() || "http://localhost:11434";
    const token = ++this.token;
    this.ollamaTest.disabled = true;
    setStatus(this.ollamaStatus, "hint", "Verbindung wird geprüft …");
    const [status, msg, names] = await invoke("ollama_check", { url });
    if (token !== this.token) return;
    this.ollamaTest.disabled = false;
    this.ollamaOk = status === "ok";
    setStatus(this.ollamaStatus, this.ollamaOk ? "success" : "error", msg);
    if (this.provider.value === "ollama") this.fillModels(names);
    if (this.ollamaOk) clearWizardError();
    this.updateOverview();
  }

  async load() {
    for (const [id, name] of META.llmProviders) {
      if (id === "custom") continue; // eigene Endpunkte nur in den Einstellungen
      this.provider.append(h("option", { value: id, text: name }));
    }
    const cfg = S.text_improvement.model;
    const model = cfg.llm_provider === "openrouter" ? S.openrouter_llm_model : cfg.llm_model;
    this.wantedModel = model || "";
    this.ollamaUrl.value = S.ollama_base_url;
    this.provider.value = cfg.llm_provider;
    if (this.provider.selectedIndex < 0) this.provider.selectedIndex = 0;
    await this.onProviderChanged();
    if (model && !["openrouter", "ollama"].includes(this.provider.value)) this.model.value = model;
  }

  async enter() {
    // Der STT-Dienst kann sich auf Seite 2 geändert haben -> Key-Anzeige anpassen
    this.sttProvider = S.transcription_model.stt_provider;
    await this.updateMode();
  }

  async canContinue() {
    if (!this.enabled.checked) return [true, ""];
    const p = this.provider.value;
    if (p === "ollama" && !this.ollamaOk) {
      return [false, "Ollama läuft nicht oder hat noch kein Modell. Bitte prüfen und „Verbindung testen“ klicken."];
    }
    if (!this.model.value.trim()) return [false, "Bitte ein Modell auswählen oder eintragen."];
    if (p in LLM_KEY_REQUIRED && !(await this.keyShared(LLM_KEY_REQUIRED[p]))) return this.keyBox.commit();
    return [true, ""];
  }

  store() {
    if (!this.enabled.checked) return;
    const p = this.provider.value;
    const model = this.model.value.trim();
    for (const cfg of llmModels(S)) {
      cfg.llm_provider = p;
      cfg.llm_model = model;
    }
    if (p === "openrouter") S.openrouter_llm_model = model;
    else if (p === "ollama") {
      S.ollama_base_url = this.ollamaUrl.value.trim() || "http://localhost:11434";
      S.ollama_llm_model = model;
    }
  }
}

// 4 ── Mikrofon

class MicrophonePage extends Page {
  constructor() {
    super("Mikrofon", "Wähle dein Mikrofon und sprich kurz hinein. Der Balken zeigt, ob BlitzVox dich hört.");
    this.peak = 0;
    this.error = null;
    this.active = false;
    this.timer = null;
    this.device = h("select");
    this.device.append(h("option", { value: "", text: "System-Standard" }));
    this.device.addEventListener("change", () => this.restartMonitor());
    this.bar = h("div");
    this.status = h("div");
    this.body.append(formRow("Aufnahmegerät:", this.device), h("div", { class: "level" }, this.bar), this.status);
  }

  async load() {
    let devices = [];
    try {
      devices = await invoke("input_devices");
    } catch (e) {
      this.error = `Audio-System nicht verfügbar: ${e}`;
    }
    for (const d of devices) {
      this.device.append(h("option", { value: d.id, text: d.name + (d.default ? "  (Standard)" : "") }));
    }
    this.device.value = S.audio_input_device_id || "";
    if (this.device.selectedIndex < 0) this.device.selectedIndex = 0;
  }

  async enter() {
    this.active = true;
    await this.restartMonitor();
  }

  leave() {
    this.active = false;
    this.stopMonitor();
  }

  stopMonitor() {
    clearInterval(this.timer);
    this.timer = null;
    invoke("mic_stop");
    this.bar.style.width = "0%";
  }

  async restartMonitor() {
    if (!this.active) return;
    this.stopMonitor();
    this.peak = 0;
    this.error = await invoke("mic_start", { device: this.device.value || null });
    if (this.error) {
      setStatus(this.status, "error", `Das Mikrofon lässt sich nicht öffnen: ${this.error}`);
      return;
    }
    setStatus(this.status, "hint", "Sprich jetzt ein paar Worte …");
    this.timer = setInterval(() => this.tick(), 50);
  }

  async tick() {
    const level = await invoke("mic_level");
    if (!this.timer) return;
    this.bar.style.width = `${Math.floor(level * 100)}%`;
    if (level > this.peak) {
      this.peak = level;
      if (this.peak > 0.15) {
        setStatus(this.status, "success", "✓ BlitzVox hört dich.");
        clearWizardError();
      }
    }
  }

  async canContinue() {
    if (this.error) return [false, "Bitte ein funktionierendes Mikrofon auswählen."];
    return [true, ""];
  }

  store() {
    S.audio_input_device_id = this.device.value || null;
  }
}

// 5 ── Tastenkürzel

const HOTKEY_ROWS = [
  ["transcription", "transcription_mode", "🎙  BlitzVox", "Diktieren und einfügen, ohne KI"],
  ["text_improvement", "text_improvement_mode", "✨  BlitzVox+", "Text verbessern – für lange Texte und freies Sprechen"],
  ["dampf_ablassen", "dampf_ablassen_mode", "🔥  BlitzVox $%&!", "Frust in eine sachliche Nachricht umformulieren"],
  ["emoji_text", "emoji_text_mode", "😊  BlitzVox :)", "Text mit passenden Emojis"],
  ["correction", null, "✏  Korrigieren", "Fenster öffnen, um den letzten Text zu korrigieren"],
];

class HotkeyPage extends Page {
  constructor() {
    super("Tastenkürzel", "Mit diesen Tastenkürzeln startest du jede Funktion aus jedem Programm heraus, ohne ein Fenster zu öffnen.");
    this.fields = {};
    this.modes = {};
    const grid = h("div", { class: "hk-grid" });
    for (const t of ["Funktion", "Tastenkürzel", "Modus"]) grid.append(h("span", { class: "label", text: t }));
    for (const [key, modeKey, name, desc] of HOTKEY_ROWS) {
      const field = hotkeyEdit("", clearWizardError);
      field.placeholder = "Feld anklicken, Tasten drücken";
      this.fields[key] = field;
      grid.append(h("span", { text: name, style: "white-space: pre" }), field);
      if (modeKey) {
        const mode = select([["hold", "Halten"], ["toggle", "Umschalten"]], "hold");
        this.modes[modeKey] = mode;
        grid.append(mode);
      } else {
        grid.append(h("span"));
      }
      grid.append(h("div", { class: "hint-line span", text: desc }));
    }
    this.body.append(grid,
      hint("Halten: sprechen, solange die Tasten gedrückt sind. Umschalten: einmal drücken zum Starten, noch einmal zum Beenden. Zum Ändern in ein Feld klicken und die neue Kombination drücken. Mit der Rücktaste wird ein Feld geleert, die Funktion ist dann nur noch über das Tray-Symbol erreichbar. Das Kürzel für BlitzVox ist Pflicht, es wird beim Probediktat getestet."));
    this.waylandWarn = h("div", { class: "error-text hidden",
      text: "Du nutzt eine Wayland-Sitzung. Globale Tastenkürzel funktionieren dort nur eingeschränkt. Melde dich für volle Funktion mit einer X11-Sitzung an (unter Linux Mint: „Cinnamon“ statt „Cinnamon on Wayland“)." });
    this.body.append(this.waylandWarn);
  }

  async load() {
    const hk = S.hotkeys;
    for (const [key, f] of Object.entries(this.fields)) f.value = hk[key];
    for (const [modeKey, combo] of Object.entries(this.modes)) combo.value = hk[modeKey] === "hold" ? "hold" : "toggle";
    this.waylandWarn.classList.toggle("hidden", !INFO.wayland);
  }

  static valid(combo) {
    const parts = combo.split("+").map((p) => p.trim()).filter(Boolean);
    const mods = new Set(["ctrl", "alt", "shift", "meta", "cmd", "win", "super"]);
    return parts.some((p) => mods.has(p)) && !parts.every((p) => mods.has(p));
  }

  async canContinue() {
    const names = Object.fromEntries(HOTKEY_ROWS.map(([key, , name]) => [key, name.split("  ").slice(1).join("  ") || name]));
    if (!this.fields.transcription.value.trim()) return [false, "Bitte ein Tastenkürzel für „BlitzVox“ festlegen."];
    const seen = {};
    for (const [key, f] of Object.entries(this.fields)) {
      const combo = f.value.trim().toLowerCase();
      if (!combo) continue;
      if (!HotkeyPage.valid(combo)) {
        return [false, `Kürzel für „${names[key]}“: Bitte eine Kombination aus Strg, Alt, Umschalt oder Win und einer weiteren Taste wählen.`];
      }
      if (combo in seen) return [false, `„${formatHotkey(combo)}“ ist doppelt vergeben: „${seen[combo]}“ und „${names[key]}“.`];
      seen[combo] = names[key];
    }
    return [true, ""];
  }

  store() {
    const hk = S.hotkeys;
    for (const [key, f] of Object.entries(this.fields)) hk[key] = f.value.trim().toLowerCase();
    for (const [modeKey, combo] of Object.entries(this.modes)) hk[modeKey] = combo.value;
    hk.enabled = true;
  }
}

// 6 ── Probediktat

class TestPage extends Page {
  constructor() {
    super("Probediktat", "Jetzt prüfen wir alles zusammen: Tastenkürzel, Mikrofon und Spracherkennung.");
    this.succeeded = false;
    this.instruction = h("div", { class: "strong" });
    this.result = h("textarea", { readonly: "", placeholder: "Hier erscheint dein diktierter Text.", style: "height: 110px; width: 100%" });
    this.button = h("button", { text: "🎙  Aufnahme starten", style: "align-self: flex-start" });
    this.button.addEventListener("click", () => this.toggleByButton());
    this.status = h("div", { class: "hidden" });
    this.body.append(this.instruction, this.result, this.button, this.status,
      hint("Klappt das Tastenkürzel nicht, kannst du auch den Knopf benutzen. Im Alltag fügt BlitzVox den Text direkt dort ein, wo dein Cursor steht."));
    listen("wizard-phase", (ev) => this.onPhase(ev.payload.phase, ev.payload.message));
    listen("wizard-hotkey", (ev) => (ev.payload === "start" ? this.start() : this.stop()));
  }

  async enter() {
    const hk = S.hotkeys;
    const name = formatHotkey(hk.transcription);
    this.instruction.textContent = hk.transcription_mode === "hold"
      ? `Halte ${name} gedrückt, sprich einen Satz und lass dann los.`
      : `Drücke ${name}, sprich einen Satz und drücke ${name} noch einmal.`;
    const err = await invoke("wizard_hotkey", { combo: hk.transcription, mode: hk.transcription_mode });
    if (err) setStatus(this.status, "hint", "Tastenkürzel sind auf diesem System nicht verfügbar. Bitte den Knopf benutzen.");
  }

  leave() {
    invoke("wizard_test_leave");
    this.button.textContent = "🎙  Aufnahme starten";
    this.button.disabled = false;
  }

  async toggleByButton() {
    if (await invoke("wizard_test_recording")) this.stop();
    else this.start();
  }

  async start() {
    if (this.button.disabled || (await invoke("wizard_test_recording"))) return;
    invoke("wizard_test_start", { settings: S });
  }

  stop() {
    invoke("wizard_test_stop");
  }

  onPhase(phase, message) {
    if (phase === "recording") {
      this.button.textContent = "⏹  Aufnahme beenden";
      setStatus(this.status, "status", "🔴  Aufnahme läuft … sprich jetzt.");
    } else if (phase === "transcribing" || phase === "rewriting") {
      this.button.textContent = "🎙  Aufnahme starten";
      this.button.disabled = true;
      setStatus(this.status, "status", message);
    } else if (phase === "done") {
      this.button.disabled = false;
      this.result.value = message;
      this.succeeded = true;
      setStatus(this.status, "success", "✓ Alles funktioniert. Du kannst noch einmal testen oder weitergehen.");
    } else if (phase === "error") {
      this.button.textContent = "🎙  Aufnahme starten";
      this.button.disabled = false;
      setStatus(this.status, "error", `${message}\nMit „Zurück“ kannst du die Einstellungen ändern.`);
    }
  }
}

// 7 ── Abschluss

class FinishPage extends Page {
  constructor() {
    super("Fast fertig", "Noch zwei Grundeinstellungen. Alles Weitere findest du jederzeit in den Einstellungen.");
    this.autoPaste = h("input", { type: "checkbox" });
    this.autostart = h("input", { type: "checkbox" });
    this.autostartLabel = h("span");
    this.pasteWarn = h("div", { class: "error-text hidden" });
    this.summary = h("div", { id: "summary" });
    this.body.append(
      h("label", { class: "check" }, this.autoPaste, h("span", { text: "Text automatisch dort einfügen, wo der Cursor steht" })),
      hint("Ohne diese Option landet der Text nur in der Zwischenablage und du fügst ihn selbst mit Strg+V ein."),
      this.pasteWarn,
      h("label", { class: "check" }, this.autostart, this.autostartLabel),
      h("div", { style: "height: 8px" }),
      h("div", { class: "label", text: "Zusammenfassung" }),
      this.summary);
  }

  async load() {
    this.autoPaste.checked = S.auto_paste;
    this.autostart.checked = S.launch_at_login;
    this.autostartLabel.textContent = META.windows ? "BlitzVox mit Windows starten" : "BlitzVox beim Anmelden starten";
    if (INFO.xdotoolMissing) {
      this.pasteWarn.textContent = "Für das automatische Einfügen fehlt das Programm xdotool. Installieren mit: sudo apt install xdotool";
      this.pasteWarn.classList.remove("hidden");
    }
  }

  async enter() {
    const code = S.transcription_model.language;
    const langEntry = META.languages.find(([, c]) => c === code);
    const lang = langEntry ? langEntry[0] : code || "—";
    const stt = displayName(META.sttProviders, S.transcription_model.stt_provider) || "—";
    const hk = S.hotkeys;
    const keyText = (key, modeKey) => {
      const combo = hk[key];
      if (!combo) return "kein Kürzel";
      if (!modeKey) return formatHotkey(combo);
      return `${formatHotkey(combo)} (${hk[modeKey] === "hold" ? "halten" : "umschalten"})`;
    };
    const llmCfg = S.text_improvement.model;
    let llm = displayName(META.llmProviders, llmCfg.llm_provider) || "—";
    if (llmCfg.llm_model) llm += ` – ${llmCfg.llm_model}`;

    const lines = [];
    const add = (label, value) => lines.push(h("b", { text: `${label}:` }), ` ${value}`, h("br"));
    add("Sprache", lang);
    add("Spracherkennung", stt);
    add("KI-Funktionen", llm);
    lines.push(h("br"), h("b", { text: "Tastenkürzel" }), h("br"));
    for (const [key, modeKey, name] of HOTKEY_ROWS) {
      lines.push(`${name.split("  ").slice(1).join("  ")}: ${keyText(key, modeKey)}`, h("br"));
    }
    this.summary.replaceChildren(...lines);
  }

  store() {
    S.auto_paste = this.autoPaste.checked;
    S.launch_at_login = this.autostart.checked;
  }
}

// ── Assistent ───────────────────────────────────────────────────────────────

const pages = [new WelcomePage(), new SpeechPage(), new AiPage(), new MicrophonePage(), new HotkeyPage(), new TestPage(), new FinishPage()];
let current = 0;
let busy = false;

const errorEl = document.getElementById("error");
const stepEl = document.getElementById("step");
const backBtn = document.getElementById("back");
const nextBtn = document.getElementById("next");

async function showPage(idx) {
  current = idx;
  pages.forEach((p, i) => p.root.classList.toggle("active", i === idx));
  document.getElementById("pages").scrollTop = 0;
  const last = idx === pages.length - 1;
  stepEl.textContent = `Schritt ${idx + 1} von ${pages.length}`;
  backBtn.disabled = idx === 0;
  nextBtn.textContent = last ? "Fertig" : "Weiter";
  errorEl.classList.add("hidden");
  await pages[idx].enter();
}

backBtn.addEventListener("click", async () => {
  if (busy || current === 0) return;
  const page = pages[current];
  page.leave();
  page.store();
  await showPage(current - 1);
});

nextBtn.addEventListener("click", async () => {
  if (busy) return;
  busy = true;
  try {
    const page = pages[current];
    const [ok, message] = await page.canContinue();
    if (!ok) {
      errorEl.textContent = message;
      errorEl.classList.remove("hidden");
      return;
    }
    page.leave();
    page.store();
    if (current < pages.length - 1) {
      await showPage(current + 1);
    } else {
      await invoke("wizard_finish", { settings: S });
    }
  } finally {
    busy = false;
  }
});

// Abbrechen: Eingaben verwerfen (beim Weiterklicken gespeicherte Keys bleiben)
document.getElementById("cancel").addEventListener("click", () => {
  pages[current].leave();
  invoke("close_window");
});

async function init() {
  const data = await invoke("wizard_init");
  S = data.settings;
  META = data.meta;
  INFO = data;
  const container = document.getElementById("pages");
  for (const p of pages) container.append(p.root);
  for (const p of pages) await p.load();
  await showPage(data.startPage || 0);
}

init().catch((e) => console.error(e));
