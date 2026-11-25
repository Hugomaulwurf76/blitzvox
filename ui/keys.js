// Tastenkürzel auch dann, wenn ein BlitzVox-Fenster den Fokus hat.
//
// Hat ein BlitzVox-Fenster den Fokus, meldet der Windows-Tastatur-Hook keine
// Tasten (beobachtet am 24.11.2026). Dieses Skript reicht die Tasten des
// Fensters deshalb an dieselbe Kürzel-Auswertung weiter. Ausnahme: Felder, in
// denen ein neues Kürzel eingegeben wird (Klasse „hk-field“).
"use strict";

(() => {
  const tauriInvoke = window.__TAURI__.core.invoke;

  /// Tastenname wie bei der Auswertung (Namen wie bei pynput in 1.0.8).
  function hotkeyName(e) {
    switch (e.key) {
      case "Control":
        return "ctrl";
      case "Alt":
      case "AltGraph":
        return "alt";
      case "Shift":
        return "shift";
      case "Meta":
      case "OS":
        return "cmd";
    }
    const c = e.keyCode;
    if (c >= 65 && c <= 90) return String.fromCharCode(c + 32);
    if (c >= 48 && c <= 57) return String.fromCharCode(c);
    if (c >= 112 && c <= 135) return "f" + (c - 111);
    const named = {
      " ": "space", Enter: "enter", Escape: "esc", Tab: "tab", Backspace: "backspace",
      Delete: "delete", Insert: "insert", Home: "home", End: "end", PageUp: "page_up",
      PageDown: "page_down", ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up",
      ArrowDown: "down", PrintScreen: "print_screen", Pause: "pause", CapsLock: "caps_lock",
      NumLock: "num_lock", ScrollLock: "scroll_lock", ContextMenu: "menu",
    };
    if (named[e.key]) return named[e.key];
    if (e.key && e.key.length === 1) return e.key.toLowerCase();
    return "";
  }

  function forward(e, down) {
    if (e.target && e.target.classList && e.target.classList.contains("hk-field")) return;
    const key = hotkeyName(e);
    if (key) tauriInvoke("key_event", { down, key }).catch(() => {});
  }

  window.addEventListener("keydown", (e) => forward(e, true), true);
  window.addEventListener("keyup", (e) => forward(e, false), true);
})();
