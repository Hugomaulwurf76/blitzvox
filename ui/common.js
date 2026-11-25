// Gemeinsame Bausteine für Einstellungen und Einrichtungsassistent.
"use strict";

/// Element erzeugen: h("div", { class: "x", text: "…" }, kind1, kind2)
function h(tag, attrs = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = v;
    else if (k === "style") e.style.cssText = v;
    else e.setAttribute(k, v);
  }
  for (const c of children) e.append(c);
  return e;
}

/// Tastenname wie QKeySequence (PortableText, klein geschrieben) in 1.0.8.
function keyName(e) {
  const c = e.keyCode;
  if (c >= 65 && c <= 90) return String.fromCharCode(c + 32);
  if (c >= 48 && c <= 57) return String.fromCharCode(c);
  if (c >= 112 && c <= 135) return "f" + (c - 111);
  const named = {
    " ": "space", Enter: "return", Escape: "esc", Tab: "tab", Backspace: "backspace",
    Delete: "del", Insert: "ins", Home: "home", End: "end", PageUp: "pgup", PageDown: "pgdown",
    ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down",
    PrintScreen: "print", Pause: "pause", CapsLock: "capslock", NumLock: "numlock",
    ScrollLock: "scrolllock", ContextMenu: "menu",
  };
  if (named[e.key]) return named[e.key];
  if (e.key && e.key.length === 1) return e.key.toLowerCase();
  return "";
}

/// Feld, das die gedrückte Kombination aufnimmt, z. B. 'ctrl+shift+g'
/// (wie HotkeyEdit in settings_window.py). `onChange` nach jeder Änderung.
function hotkeyEdit(value, onChange) {
  const f = h("input", { type: "text", class: "hk-field", readonly: "" });
  f.value = value || "";
  f.addEventListener("keydown", (e) => {
    e.preventDefault();
    if (["Control", "Shift", "Alt", "Meta", "AltGraph", "OS"].includes(e.key)) return; // nur Modifier: noch nichts
    const mods = e.ctrlKey || e.altKey || e.shiftKey || e.metaKey;
    if ((e.key === "Backspace" || e.key === "Delete") && !mods) {
      f.value = "";
      if (onChange) onChange();
      return;
    }
    const k = keyName(e);
    if (!k) return;
    const parts = [];
    if (e.ctrlKey) parts.push("ctrl");
    if (e.altKey) parts.push("alt");
    if (e.shiftKey) parts.push("shift");
    if (e.metaKey) parts.push("meta");
    parts.push(k);
    f.value = parts.join("+");
    if (onChange) onChange();
  });
  return f;
}

/// Auswahlliste aus [Wert, Anzeige]-Paaren.
function select(options, current) {
  const s = h("select");
  for (const [value, label] of options) s.append(h("option", { value, text: label }));
  s.value = current;
  if (s.selectedIndex < 0) s.selectedIndex = 0;
  return s;
}

/// Editierbare Auswahlliste wie QComboBox(editable) in 1.0.8: Eingabefeld plus
/// Pfeil-Knopf. Der Pfeil zeigt alle Einträge, beim Tippen erscheinen die
/// passenden. (Die eingebaute datalist des Browsers zeigt nur Einträge, die
/// zum Feldinhalt passen – ein gefülltes Feld wirkte dann wie eine leere Liste.)
function editableCombo() {
  const input = h("input", { type: "text", autocomplete: "off" });
  const arrow = h("button", { class: "combo-arrow", type: "button", tabindex: "-1", "aria-label": "Liste öffnen" });
  const list = h("div", { class: "combo-list hidden" });
  const wrap = h("div", { class: "combo" }, input, arrow, list);
  let items = [];

  const close = () => list.classList.add("hidden");
  const open = (filter) => {
    const f = (filter || "").toLowerCase();
    const shown = f ? items.filter((i) => i.toLowerCase().includes(f)) : items;
    list.replaceChildren(
      ...shown.slice(0, 500).map((i) => {
        const row = h("div", { class: "combo-item" + (i === input.value ? " current" : ""), text: i });
        row.addEventListener("mousedown", (e) => {
          e.preventDefault();
          input.value = i;
          close();
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
        });
        return row;
      }),
    );
    list.classList.toggle("hidden", shown.length === 0 || input.disabled);
    const cur = list.querySelector(".current");
    if (cur) cur.scrollIntoView({ block: "nearest" });
  };

  input.setItems = (newItems) => {
    items = [...newItems];
    if (!list.classList.contains("hidden")) open("");
  };
  arrow.addEventListener("mousedown", (e) => {
    e.preventDefault();
    if (input.disabled) return;
    if (list.classList.contains("hidden")) {
      input.focus();
      open("");
    } else close();
  });
  input.addEventListener("input", (e) => {
    if (e.isTrusted) open(input.value);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") close();
  });
  input.addEventListener("blur", () => setTimeout(close, 100));
  // Gesperrtes Feld: Pfeil ebenfalls sperren
  new MutationObserver(() => (arrow.disabled = input.disabled)).observe(input, { attributes: true, attributeFilter: ["disabled"] });
  return { input, wrap };
}

/// Fenster „Key anzeigen“ (wie key_popup.py): ohne Rahmen, ✕ oder Esc schließt.
/// Braucht im HTML #key-overlay, #key-title, #key-text, #key-close.
function showKey(title, key) {
  document.getElementById("key-title").textContent = title;
  document.getElementById("key-text").textContent = key;
  document.getElementById("key-overlay").classList.remove("hidden");
}
function hideKey() {
  document.getElementById("key-overlay").classList.add("hidden");
  document.getElementById("key-text").textContent = "";
}
document.addEventListener("DOMContentLoaded", () => {
  const close = document.getElementById("key-close");
  if (close) close.addEventListener("click", hideKey);
});
document.addEventListener("keydown", (e) => {
  const o = document.getElementById("key-overlay");
  if (e.key === "Escape" && o && !o.classList.contains("hidden")) hideKey();
});
