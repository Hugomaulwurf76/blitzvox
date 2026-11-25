//! Globale Tastenkürzel mit Halten- und Umschalt-Modus – 1:1-Nachbau von
//! `BlitzVox alt/blitzvox/services/hotkeys.py` (dort mit pynput).
//!
//! Wie pynput hört BlitzVox jeden Tastendruck systemweit mit und wertet die
//! Kombinationen selbst aus. Die Tasten werden dabei nicht abgefangen, das
//! aktive Programm bekommt sie weiterhin.
//!
//! Halten:   Alle Tasten der Kombi gedrückt  -> `Pressed`  (Aufnahme starten)
//!           Eine Taste der Kombi losgelassen -> `Released` (Aufnahme stoppen)
//! Umschalt: Kombi einmal vollständig gedrückt -> `Toggled(an/aus)`
//!
//! Windows: Tastatur-Hook (`WH_KEYBOARD_LL`). Linux/X11: Erweiterung RECORD
//! (wie pynput). Linux/Wayland (neu in 2.0, 1.0.8 konnte das nicht): zuerst
//! Tastatur direkt über /dev/input lesen, sonst Portal „GlobalShortcuts“,
//! zuletzt RECORD über XWayland (wirkt dann nur in X11-Programmen).

// Wird erst mit Tray und Workflows vollständig benutzt.
#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::settings::HotkeyConfig;

// ── Tasten ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Cmd,
}

/// Eine erkannte Taste, wie sie `_mod_name()` bzw. `_key_char()` liefern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Modifier(Modifier),
    /// Normale Taste: Kleinbuchstabe, Ziffer, Zeichen oder Name wie
    /// `f5`, `space`, `enter`, `esc` (Namen wie bei pynput)
    Normal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub down: bool,
    pub key: Key,
}

fn normalize(part: &str) -> String {
    let p = part.trim().to_lowercase();
    match p.as_str() {
        "control" => "ctrl".into(),
        "option" => "alt".into(),
        "win" | "super" | "meta" => "cmd".into(),
        "return" => "enter".into(),
        "esc" => "escape".into(),
        _ => p,
    }
}

/// Zerlegt `ctrl+shift+r` in (Modifier, normale Taste) wie `_parse_combo()`.
pub fn parse_combo(combo: &str) -> (HashSet<Modifier>, Option<String>) {
    let mut mods = HashSet::new();
    let mut key_char = None;
    for raw in combo.split('+') {
        match normalize(raw).as_str() {
            "ctrl" => {
                mods.insert(Modifier::Ctrl);
            }
            "shift" => {
                mods.insert(Modifier::Shift);
            }
            "alt" => {
                mods.insert(Modifier::Alt);
            }
            "cmd" => {
                mods.insert(Modifier::Cmd);
            }
            other => key_char = Some(other.to_string()),
        }
    }
    (mods, key_char)
}

// ── Zustandsautomat (wie HotkeyService._on_key_press/_release/_evaluate) ─────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Hold,
    Toggle,
}

impl Mode {
    /// Wie in tray.py: „hold“ ist Halten, alles andere Umschalten.
    pub fn from_setting(mode: &str) -> Self {
        if mode == "hold" { Mode::Hold } else { Mode::Toggle }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// Halten: Kombi vollständig gedrückt
    Pressed,
    /// Halten: eine Taste der Kombi losgelassen
    Released,
    /// Umschalten: neuer Zustand (true = an)
    Toggled(bool),
}

type Handler = Arc<dyn Fn(HotkeyEvent) + Send + Sync>;

struct Binding {
    mods: HashSet<Modifier>,
    key_char: Option<String>,
    mode: Mode,
    handler: Handler,
    /// Ist die Kombi gerade vollständig gedrückt?
    active: bool,
    /// Umschalten: läuft gerade eine Aufnahme?
    toggle_state: bool,
}

#[derive(Default)]
struct Engine {
    bindings: Vec<Binding>,
    pressed_mods: HashSet<Modifier>,
    pressed_keys: HashSet<String>,
}

impl Engine {
    /// Liefert die auszulösenden Ereignisse als (Index der Bindung, Ereignis).
    fn handle(&mut self, ev: &KeyEvent) -> Vec<(usize, HotkeyEvent)> {
        if ev.down { self.on_press(&ev.key) } else { self.on_release(&ev.key) }
    }

    fn on_press(&mut self, key: &Key) -> Vec<(usize, HotkeyEvent)> {
        match key {
            Key::Modifier(m) => {
                self.pressed_mods.insert(*m);
            }
            Key::Normal(c) => {
                self.pressed_keys.insert(c.clone());
            }
        }
        self.evaluate()
    }

    fn on_release(&mut self, key: &Key) -> Vec<(usize, HotkeyEvent)> {
        let mut out = Vec::new();
        // Zur Fehlersuche: Loslassen einer Taste, die zu einem Kürzel gehört
        if let Key::Normal(c) = key {
            if self.bindings.iter().any(|b| b.key_char.as_deref() == Some(c.as_str())) {
                log::debug!("Kürzeltaste „{c}“ losgelassen");
            }
        }
        // Vor dem Entfernen prüfen: war eine Halten-Kombi aktiv und wird jetzt gebrochen?
        for (i, b) in self.bindings.iter_mut().enumerate() {
            if b.mode == Mode::Hold && b.active {
                let breaking = match key {
                    Key::Modifier(m) => b.mods.contains(m),
                    Key::Normal(c) => b.key_char.as_deref() == Some(c.as_str()),
                };
                if breaking {
                    b.active = false;
                    out.push((i, HotkeyEvent::Released));
                }
            }
        }
        match key {
            Key::Modifier(m) => {
                self.pressed_mods.remove(m);
            }
            Key::Normal(c) => {
                self.pressed_keys.remove(c);
            }
        }
        out
    }

    /// Prüft, ob eine Kombi gerade vollständig gedrückt ist.
    fn evaluate(&mut self) -> Vec<(usize, HotkeyEvent)> {
        let mut out = Vec::new();
        for (i, b) in self.bindings.iter_mut().enumerate() {
            let combo_down = b.mods.is_subset(&self.pressed_mods)
                && b.key_char.as_ref().is_some_and(|k| self.pressed_keys.contains(k));
            if combo_down && !b.active {
                b.active = true;
                match b.mode {
                    Mode::Hold => out.push((i, HotkeyEvent::Pressed)),
                    Mode::Toggle => {
                        b.toggle_state = !b.toggle_state;
                        out.push((i, HotkeyEvent::Toggled(b.toggle_state)));
                    }
                }
            } else if !combo_down && b.active && b.mode == Mode::Toggle {
                // Umschalten: active zurücksetzen, sobald die Kombi nicht mehr
                // gedrückt ist, damit erneutes Drücken wieder auslöst
                b.active = false;
            }
        }
        out
    }
}

// ── Dienst ───────────────────────────────────────────────────────────────────

/// Sammelt Tastenkürzel und hört nach `start()` systemweit mit.
pub struct HotkeyService {
    bindings: Vec<Binding>,
    subscription: Option<u64>,
}

impl Default for HotkeyService {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyService {
    pub fn new() -> Self {
        Self { bindings: Vec::new(), subscription: None }
    }

    /// Meldet eine Kombination an. Leere Kombinationen werden ignoriert.
    pub fn register(
        &mut self,
        combo: &str,
        mode: Mode,
        handler: impl Fn(HotkeyEvent) + Send + Sync + 'static,
    ) {
        if combo.is_empty() {
            return;
        }
        let (mods, key_char) = parse_combo(combo);
        self.bindings.push(Binding {
            mods,
            key_char,
            mode,
            handler: Arc::new(handler),
            active: false,
            toggle_state: false,
        });
    }

    /// Startet das Mithören. Ohne Bindungen passiert nichts.
    pub fn start(&mut self) -> Result<(), String> {
        if self.subscription.is_some() || self.bindings.is_empty() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel::<KeyEvent>();
        let id = listener::subscribe(tx)?;
        let engine = Engine { bindings: std::mem::take(&mut self.bindings), ..Default::default() };
        thread::Builder::new()
            .name("blitzvox-hotkeys".into())
            .spawn(move || run_engine(engine, rx))
            .map_err(|e| e.to_string())?;
        self.subscription = Some(id);
        Ok(())
    }

    /// Beendet das Mithören.
    pub fn stop(&mut self) {
        if let Some(id) = self.subscription.take() {
            listener::unsubscribe(id);
        }
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Läuft, bis der Sender beim Abmelden entfernt wird.
fn run_engine(mut engine: Engine, rx: Receiver<KeyEvent>) {
    for ev in rx {
        for (i, event) in engine.handle(&ev) {
            let handler = engine.bindings[i].handler.clone();
            // Ein Fehler im Callback darf das Mithören nicht beenden
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event))).is_err() {
                log::error!("[HotkeyService] callback error");
            }
        }
    }
}

// ── Anbindung an die Einstellungen (wie TrayApp._setup_hotkeys) ──────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyWorkflow {
    Transcription,
    TextImprovement,
    DampfAblassen,
    EmojiText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Fenster öffnen (falls nötig) und Aufnahme starten
    Start(HotkeyWorkflow),
    /// Aufnahme beenden und verarbeiten
    Stop(HotkeyWorkflow),
    /// Fenster „Letzten Text korrigieren“ öffnen
    Correction,
}

/// Richtet die Tastenkürzel aus den Einstellungen ein und startet sie.
/// `None`, wenn die Tastenkürzel abgeschaltet sind.
pub fn start_from_settings(
    hk: &HotkeyConfig,
    on_action: impl Fn(HotkeyAction) + Send + Sync + 'static,
) -> Result<Option<HotkeyService>, String> {
    if !hk.enabled {
        return Ok(None);
    }
    let on_action: Arc<dyn Fn(HotkeyAction) + Send + Sync> = Arc::new(on_action);
    let mut svc = HotkeyService::new();
    let mapping = [
        (&hk.transcription, &hk.transcription_mode, HotkeyWorkflow::Transcription),
        (&hk.text_improvement, &hk.text_improvement_mode, HotkeyWorkflow::TextImprovement),
        (&hk.dampf_ablassen, &hk.dampf_ablassen_mode, HotkeyWorkflow::DampfAblassen),
        (&hk.emoji_text, &hk.emoji_text_mode, HotkeyWorkflow::EmojiText),
    ];
    for (combo, mode, wf) in mapping {
        if combo.is_empty() {
            continue;
        }
        let f = on_action.clone();
        svc.register(combo, Mode::from_setting(mode), move |ev| match ev {
            HotkeyEvent::Pressed | HotkeyEvent::Toggled(true) => f(HotkeyAction::Start(wf)),
            HotkeyEvent::Released | HotkeyEvent::Toggled(false) => f(HotkeyAction::Stop(wf)),
        });
    }
    // Korrekturfenster: jeder Tastendruck öffnet es (Umschalt-Zustand egal)
    if !hk.correction.is_empty() {
        let f = on_action.clone();
        svc.register(&hk.correction, Mode::Toggle, move |_| f(HotkeyAction::Correction));
    }
    svc.start()?;
    // Wayland-Portal: Der Desktop muss die Kürzel selbst kennen.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (combo, desc) in [
        (&hk.transcription, "BlitzVox: Nur Transkription"),
        (&hk.text_improvement, "BlitzVox+: Text verbessern"),
        (&hk.dampf_ablassen, "BlitzVox $%&!: Dampf ablassen"),
        (&hk.emoji_text, "BlitzVox :): Emojis"),
        (&hk.correction, "BlitzVox: Letzten Text korrigieren"),
    ] {
        if !combo.is_empty() {
            wanted.push((desc.to_string(), combo.clone()));
        }
    }
    platform::set_combos(wanted);
    Ok(Some(svc))
}

// ── Tasten aus den eigenen Fenstern ──────────────────────────────────────────

/// Tastenereignis aus einem BlitzVox-Fenster einspeisen.
///
/// Hat ein BlitzVox-Fenster den Fokus, meldet der Windows-Hook keine Tasten
/// (beobachtet am 24.11.2026; Ursache in Windows/WebView2 nicht geklärt). Die
/// Fenster reichen ihre Tastenereignisse deshalb selbst weiter (`ui/keys.js`).
/// `name` wie bei pynput: „ctrl“, „alt“, „shift“, „cmd“ oder Tastenname.
pub fn inject_key(down: bool, name: &str) {
    if let Some(key) = key_from_name(name) {
        listener::broadcast(KeyEvent { down, key });
    }
}

/// Taste zu einem Namen wie bei pynput („ctrl“, „alt“, „shift“, „cmd“ oder Tastenname).
fn key_from_name(name: &str) -> Option<Key> {
    Some(match name {
        "ctrl" => Key::Modifier(Modifier::Ctrl),
        "alt" => Key::Modifier(Modifier::Alt),
        "shift" => Key::Modifier(Modifier::Shift),
        "cmd" => Key::Modifier(Modifier::Cmd),
        "" => return None,
        other => Key::Normal(other.to_lowercase()),
    })
}

/// Grund, falls die Tastenkürzel nur eingeschränkt funktionieren (Linux/Wayland
/// ohne Tastaturzugriff und ohne Portal). Sonst `None`.
pub fn limited_reason() -> Option<String> {
    platform::limited_reason()
}

// ── Gemeinsamer Listener: ein Hook für alle Dienste ──────────────────────────

mod listener {
    use super::*;

    struct Subscribers {
        next_id: u64,
        list: Vec<(u64, Sender<KeyEvent>)>,
        started: bool,
    }

    static SUBSCRIBERS: Mutex<Subscribers> =
        Mutex::new(Subscribers { next_id: 1, list: Vec::new(), started: false });

    fn lock() -> std::sync::MutexGuard<'static, Subscribers> {
        SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Verteilt ein Tastenereignis an alle angemeldeten Dienste.
    pub(super) fn broadcast(ev: KeyEvent) {
        let mut s = lock();
        let before = s.list.len();
        s.list.retain(|(_, tx)| tx.send(ev.clone()).is_ok());
        if s.list.len() != before {
            log::warn!("Tastenkürzel: {} Empfänger entfernt", before - s.list.len());
        }
    }

    pub(super) fn subscribe(tx: Sender<KeyEvent>) -> Result<u64, String> {
        let mut s = lock();
        if !s.started {
            super::platform::start_listener()?;
            s.started = true;
            log::debug!("Tastatur-Listener gestartet");
        }
        let id = s.next_id;
        s.next_id += 1;
        s.list.push((id, tx));
        log::debug!("Tastenkürzel-Dienst {id} angemeldet ({} aktiv)", s.list.len());
        Ok(id)
    }

    pub(super) fn unsubscribe(id: u64) {
        let mut s = lock();
        s.list.retain(|(i, _)| *i != id);
        log::debug!("Tastenkürzel-Dienst {id} abgemeldet ({} aktiv)", s.list.len());
    }
}

// ── Windows: Tastatur-Hook ───────────────────────────────────────────────────

#[cfg(windows)]
mod platform {
    use super::{Key, KeyEvent, Modifier, listener};
    use std::sync::mpsc;
    use std::thread;

    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, MapVirtualKeyExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW, TranslateMessage,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const MAPVK_VK_TO_CHAR: u32 = 2;

    /// Tastenname wie bei pynput (`_mod_name` / `_key_char`).
    pub(super) fn key_from_vk(vk: u32) -> Option<Key> {
        let m = |m| Some(Key::Modifier(m));
        let n = |s: &str| Some(Key::Normal(s.to_string()));
        match vk {
            0x11 | 0xA2 | 0xA3 => m(Modifier::Ctrl),
            0x10 | 0xA0 | 0xA1 => m(Modifier::Shift),
            0x12 | 0xA4 | 0xA5 => m(Modifier::Alt),
            0x5B | 0x5C => m(Modifier::Cmd),
            0x41..=0x5A => Some(Key::Normal(char::from(vk as u8 + 32).to_string())),
            0x30..=0x39 => Some(Key::Normal(char::from(vk as u8).to_string())),
            0x70..=0x87 => Some(Key::Normal(format!("f{}", vk - 0x6F))),
            0x20 => n("space"),
            0x08 => n("backspace"),
            0x14 => n("caps_lock"),
            0x2E => n("delete"),
            0x28 => n("down"),
            0x23 => n("end"),
            0x0D => n("enter"),
            0x1B => n("esc"),
            0x24 => n("home"),
            0x25 => n("left"),
            0x22 => n("page_down"),
            0x21 => n("page_up"),
            0x27 => n("right"),
            0x09 => n("tab"),
            0x26 => n("up"),
            0x2D => n("insert"),
            0x5D => n("menu"),
            0x90 => n("num_lock"),
            0x13 => n("pause"),
            0x2C => n("print_screen"),
            0x91 => n("scroll_lock"),
            0xB3 => n("media_play_pause"),
            0xB2 => n("media_stop"),
            0xAD => n("media_volume_mute"),
            0xAE => n("media_volume_down"),
            0xAF => n("media_volume_up"),
            0xB1 => n("media_previous"),
            0xB0 => n("media_next"),
            _ => char_for_vk(vk).map(Key::Normal),
        }
    }

    /// Übrige Tasten: Zeichen der Taste im aktuellen Tastaturlayout
    /// (ohne Umschalt/Strg), klein geschrieben.
    fn char_for_vk(vk: u32) -> Option<String> {
        // SAFETY: reine Abfragen ohne Zeiger-Parameter (bis auf null).
        let ch = unsafe {
            let tid = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
            MapVirtualKeyExW(vk, MAPVK_VK_TO_CHAR, GetKeyboardLayout(tid))
        };
        let ch = ch & 0x7FFF_FFFF; // oberstes Bit = Tottaste
        if ch == 0 {
            return None;
        }
        char::from_u32(ch).map(|c| c.to_lowercase().collect())
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: Bei WH_KEYBOARD_LL zeigt lparam auf ein KBDLLHOOKSTRUCT.
            let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let down = match wparam as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            let key = key_from_vk(info.vkCode);
            // Zur Fehlersuche (nur Entwicklungsbuild): Sondertasten protokollieren,
            // keine Buchstaben. Bit 0x10 = künstlich erzeugt (LLKHF_INJECTED).
            if cfg!(debug_assertions) && matches!(key, Some(Key::Modifier(_))) {
                log::debug!(
                    "Sondertaste vk={:#x} {} {}",
                    info.vkCode,
                    if down == Some(true) { "gedrückt" } else { "losgelassen" },
                    if info.flags & 0x10 != 0 { "(künstlich)" } else { "(echt)" }
                );
            }
            if let (Some(down), Some(key)) = (down, key) {
                listener::broadcast(KeyEvent { down, key });
            }
        }
        // Taste immer weiterreichen – BlitzVox hört nur mit
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    pub(super) fn set_combos(_combos: Vec<(String, String)>) {}

    pub(super) fn limited_reason() -> Option<String> {
        None
    }

    /// Startet einen Thread mit Hook und Nachrichtenschleife (bleibt bis
    /// Programmende aktiv; abgemeldete Dienste bekommen nichts mehr).
    pub(super) fn start_listener() -> Result<(), String> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        thread::Builder::new()
            .name("blitzvox-keyboard-hook".into())
            .spawn(move || {
                // SAFETY: Hook wird im eigenen Thread mit Nachrichtenschleife gesetzt.
                let hook = unsafe {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0)
                };
                if hook.is_null() {
                    let _ = ready_tx.send(Err("Tastatur-Hook konnte nicht gesetzt werden.".into()));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                // SAFETY: Standard-Nachrichtenschleife; nötig, damit der Hook Ereignisse bekommt.
                unsafe {
                    let mut msg: MSG = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())?
    }
}

// ── Linux: X11 RECORD ────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    use super::{Key, KeyEvent, Modifier, listener};
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::thread;

    use x11rb::connection::{Connection, RequestConnection};
    use x11rb::protocol::record::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{self, ConnectionExt as _};
    use x11rb::x11_utils::TryParse;

    /// Tastenname zum Keysym (erste Ebene der Taste), wie bei pynput.
    pub(super) fn key_from_keysym(sym: u32) -> Option<Key> {
        let m = |m| Some(Key::Modifier(m));
        let n = |s: &str| Some(Key::Normal(s.to_string()));
        match sym {
            0xffe3 | 0xffe4 => m(Modifier::Ctrl),
            0xffe1 | 0xffe2 => m(Modifier::Shift),
            0xffe9 | 0xffea | 0xfe03 | 0xff7e => m(Modifier::Alt), // Alt, AltGr
            0xffeb | 0xffec => m(Modifier::Cmd),                   // Super
            0x20 => n("space"),
            0xffbe..=0xffd5 => Some(Key::Normal(format!("f{}", sym - 0xffbd))),
            0xff08 => n("backspace"),
            0xffe5 => n("caps_lock"),
            0xffff => n("delete"),
            0xff54 => n("down"),
            0xff57 => n("end"),
            0xff0d => n("enter"),
            0xff1b => n("esc"),
            0xff50 => n("home"),
            0xff51 => n("left"),
            0xff56 => n("page_down"),
            0xff55 => n("page_up"),
            0xff53 => n("right"),
            0xff09 => n("tab"),
            0xff52 => n("up"),
            0xff63 => n("insert"),
            0xff67 => n("menu"),
            0xff7f => n("num_lock"),
            0xff13 => n("pause"),
            0xff61 => n("print_screen"),
            0xff14 => n("scroll_lock"),
            // Latin-1: Keysym = Unicode-Zeichen
            0x21..=0x7e | 0xa0..=0xff => {
                char::from_u32(sym).map(|c| Key::Normal(c.to_lowercase().collect()))
            }
            _ => None,
        }
    }

    fn keymap(conn: &impl Connection) -> Result<HashMap<u8, u32>, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per = reply.keysyms_per_keycode as usize;
        let mut map = HashMap::new();
        if per == 0 {
            return Ok(map);
        }
        for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
            if let Some(&sym) = chunk.first() {
                map.insert(min + i as u8, sym);
            }
        }
        Ok(map)
    }

    // ── Wayland ──────────────────────────────────────────────────────────────

    use crate::linux_input::{self, PortalHandle, PortalShortcut};
    use std::sync::{Arc, Mutex};

    /// Portal-Verbindung und die zuletzt gebundenen Kürzel (nur im Portal-Betrieb).
    static PORTAL: Mutex<Option<(PortalHandle, Vec<String>)>> = Mutex::new(None);
    /// Grund, falls nur der eingeschränkte X11-Weg läuft.
    static LIMITED: Mutex<Option<String>> = Mutex::new(None);

    fn portal_lock() -> std::sync::MutexGuard<'static, Option<(PortalHandle, Vec<String>)>> {
        PORTAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn limited_reason() -> Option<String> {
        LIMITED.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Auslöser im Format der XDG-Spezifikation „shortcuts“, z. B. `CTRL+ALT+w`.
    fn portal_trigger(combo: &str) -> String {
        let (mods, key) = super::parse_combo(combo);
        let mut parts = Vec::new();
        for (m, name) in [
            (Modifier::Ctrl, "CTRL"),
            (Modifier::Alt, "ALT"),
            (Modifier::Shift, "SHIFT"),
            (Modifier::Cmd, "LOGO"),
        ] {
            if mods.contains(&m) {
                parts.push(name.to_string());
            }
        }
        if let Some(k) = key {
            let sym = match k.as_str() {
                "enter" => "Return".to_string(),
                "escape" | "esc" => "Escape".to_string(),
                "tab" => "Tab".to_string(),
                "backspace" => "BackSpace".to_string(),
                "delete" => "Delete".to_string(),
                "insert" => "Insert".to_string(),
                "home" => "Home".to_string(),
                "end" => "End".to_string(),
                "page_up" => "Prior".to_string(),
                "page_down" => "Next".to_string(),
                "up" => "Up".to_string(),
                "down" => "Down".to_string(),
                "left" => "Left".to_string(),
                "right" => "Right".to_string(),
                f if f.len() > 1 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
                    f.to_uppercase()
                }
                other => other.to_string(),
            };
            parts.push(sym);
        }
        parts.join("+")
    }

    pub(super) fn set_combos(combos: Vec<(String, String)>) {
        let mut guard = portal_lock();
        let Some((handle, stored)) = guard.as_mut() else { return };
        *stored = combos.iter().map(|(_, c)| c.clone()).collect();
        handle.bind(
            combos
                .iter()
                .map(|(d, c)| PortalShortcut { description: d.clone(), trigger: portal_trigger(c) })
                .collect(),
        );
    }

    /// Das Portal meldet ein Kürzel als aktiv/inaktiv: als Tastendrücke
    /// weitergeben, damit Halten und Umschalten wie sonst ausgewertet werden.
    fn on_portal(index: usize, active: bool) {
        let combo = match portal_lock().as_ref().and_then(|(_, list)| list.get(index).cloned()) {
            Some(c) => c,
            None => return,
        };
        let (mods, key) = super::parse_combo(&combo);
        let mods: Vec<Key> = mods.into_iter().map(Key::Modifier).collect();
        let key = key.map(Key::Normal);
        if active {
            for m in &mods {
                listener::broadcast(KeyEvent { down: true, key: m.clone() });
            }
            if let Some(k) = &key {
                listener::broadcast(KeyEvent { down: true, key: k.clone() });
            }
        } else {
            if let Some(k) = &key {
                listener::broadcast(KeyEvent { down: false, key: k.clone() });
            }
            for m in &mods {
                listener::broadcast(KeyEvent { down: false, key: m.clone() });
            }
        }
    }

    /// Tastaturbelegung von X11/XWayland (falls vorhanden), damit Buchstaben
    /// dem eingestellten Layout entsprechen (z. B. Y/Z bei deutscher Tastatur).
    fn x_keymap() -> Option<HashMap<u8, u32>> {
        let (conn, _) = x11rb::connect(None).ok()?;
        keymap(&conn).ok()
    }

    pub(super) fn start_listener() -> Result<(), String> {
        if !linux_input::session_is_wayland() {
            return start_record();
        }
        // 1. Tastatur direkt lesen – alle Desktops, Halten und Umschalten
        let xmap = x_keymap();
        let on_key: linux_input::KeyCallback = Arc::new(move |down, code| {
            let key = xmap
                .as_ref()
                .and_then(|m| u8::try_from(code + 8).ok().and_then(|c| m.get(&c).copied()))
                .and_then(key_from_keysym)
                .or_else(|| linux_input::us_key_name(code).and_then(super::key_from_name));
            if let Some(key) = key {
                listener::broadcast(KeyEvent { down, key });
            }
        });
        let evdev_err = match linux_input::start_keyboard_reader(on_key) {
            Ok(n) => {
                log::info!("Tastenkürzel (Wayland): {n} Tastatur(en) werden direkt gelesen");
                return Ok(());
            }
            Err(e) => {
                log::warn!("Tastenkürzel (Wayland): {e}");
                e
            }
        };
        // 2. Portal „GlobalShortcuts“ (KDE Plasma 6, GNOME ab 48 …)
        match linux_input::start_portal(Arc::new(on_portal)) {
            Ok(handle) => {
                *portal_lock() = Some((handle, Vec::new()));
                log::info!("Tastenkürzel (Wayland): über das Portal GlobalShortcuts");
                return Ok(());
            }
            Err(e) => log::warn!("Tastenkürzel (Wayland), Portal: {e}"),
        }
        // 3. Nur X11-Programme (XWayland) – wie 1.0.8
        start_record().map_err(|e| format!("{evdev_err} {e}"))?;
        log::warn!("Tastenkürzel (Wayland): nur über XWayland, wirken nur in X11-Programmen");
        *LIMITED.lock().unwrap_or_else(|e| e.into_inner()) = Some(evdev_err);
        Ok(())
    }

    // ── X11 ──────────────────────────────────────────────────────────────────

    fn start_record() -> Result<(), String> {
        let (ctrl, _) = x11rb::connect(None).map_err(|e| {
            format!("Keine Verbindung zum X-Server ({e}). Tastenkürzel brauchen X11.")
        })?;
        let (data, _) = x11rb::connect(None).map_err(|e| e.to_string())?;
        if ctrl
            .extension_information(record::X11_EXTENSION_NAME)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Err("Der X-Server unterstützt die Erweiterung RECORD nicht.".into());
        }
        let keymap = keymap(&ctrl)?;
        let rc = ctrl.generate_id().map_err(|e| e.to_string())?;
        let empty = record::Range8 { first: 0, last: 0 };
        let empty_ext = record::ExtRange { major: empty, minor: record::Range16 { first: 0, last: 0 } };
        let range = record::Range {
            core_requests: empty,
            core_replies: empty,
            ext_requests: empty_ext,
            ext_replies: empty_ext,
            delivered_events: empty,
            device_events: record::Range8 {
                first: xproto::KEY_PRESS_EVENT,
                last: xproto::KEY_RELEASE_EVENT,
            },
            errors: empty,
            client_started: false,
            client_died: false,
        };
        ctrl.record_create_context(rc, 0, &[record::CS::ALL_CLIENTS.into()], &[range])
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;

        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        thread::Builder::new()
            .name("blitzvox-x11-record".into())
            .spawn(move || {
                let _ctrl = ctrl; // Steuerverbindung muss offen bleiben
                const START_OF_DATA: u8 = 4;
                const RECORD_FROM_SERVER: u8 = 0;
                let replies = match data.record_enable_context(rc) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let mut ready_tx = Some(ready_tx);
                for reply in replies {
                    let Ok(reply) = reply else { break };
                    if reply.category == START_OF_DATA {
                        if let Some(tx) = ready_tx.take() {
                            let _ = tx.send(Ok(()));
                        }
                        continue;
                    }
                    if reply.category != RECORD_FROM_SERVER || reply.client_swapped {
                        continue;
                    }
                    let mut rest: &[u8] = &reply.data;
                    while rest.len() >= 32 {
                        let kind = rest[0] & 0x7f;
                        let down = match kind {
                            xproto::KEY_PRESS_EVENT => true,
                            xproto::KEY_RELEASE_EVENT => false,
                            _ => {
                                rest = &rest[32..];
                                continue;
                            }
                        };
                        let Ok((ev, r)) = xproto::KeyPressEvent::try_parse(rest) else { break };
                        rest = r;
                        if let Some(key) = keymap.get(&ev.detail).and_then(|&s| key_from_keysym(s)) {
                            listener::broadcast(KeyEvent { down, key });
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())?
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn set_combos(_combos: Vec<(String, String)>) {}

    pub(super) fn limited_reason() -> Option<String> {
        None
    }

    pub(super) fn start_listener() -> Result<(), String> {
        Err("Globale Tastenkürzel werden auf diesem System nicht unterstützt.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: &str) -> KeyEvent {
        KeyEvent { down: true, key: key(c) }
    }
    fn release(c: &str) -> KeyEvent {
        KeyEvent { down: false, key: key(c) }
    }
    fn key(c: &str) -> Key {
        match c {
            "ctrl" => Key::Modifier(Modifier::Ctrl),
            "alt" => Key::Modifier(Modifier::Alt),
            "shift" => Key::Modifier(Modifier::Shift),
            other => Key::Normal(other.into()),
        }
    }

    fn engine(bindings: &[(&str, Mode)]) -> Engine {
        let mut e = Engine::default();
        for (combo, mode) in bindings {
            let (mods, key_char) = parse_combo(combo);
            e.bindings.push(Binding {
                mods,
                key_char,
                mode: *mode,
                handler: Arc::new(|_| {}),
                active: false,
                toggle_state: false,
            });
        }
        e
    }

    fn run(e: &mut Engine, evs: &[KeyEvent]) -> Vec<(usize, HotkeyEvent)> {
        evs.iter().flat_map(|ev| e.handle(ev)).collect()
    }

    #[test]
    fn kombination_zerlegen() {
        let (mods, k) = parse_combo("Control+Shift+R");
        assert_eq!(mods, HashSet::from([Modifier::Ctrl, Modifier::Shift]));
        assert_eq!(k.as_deref(), Some("r"));
        assert_eq!(parse_combo("meta+return").1.as_deref(), Some("enter"));
        assert_eq!(parse_combo("ctrl+alt").1, None);
    }

    #[test]
    fn halten_start_und_stopp_bei_beliebiger_taste() {
        let mut e = engine(&[("ctrl+alt+w", Mode::Hold)]);
        let out = run(&mut e, &[press("ctrl"), press("alt"), press("w"), press("w")]);
        assert_eq!(out, vec![(0, HotkeyEvent::Pressed)]); // Tastenwiederholung löst nicht erneut aus
        // Strg zuerst loslassen beendet die Aufnahme
        assert_eq!(run(&mut e, &[release("ctrl")]), vec![(0, HotkeyEvent::Released)]);
        assert!(run(&mut e, &[release("w"), release("alt")]).is_empty());
    }

    #[test]
    fn umschalten_wechselt_und_braucht_neuen_druck() {
        let mut e = engine(&[("ctrl+alt+b", Mode::Toggle)]);
        let ein = run(&mut e, &[press("ctrl"), press("alt"), press("b"), release("b"), release("alt"), release("ctrl")]);
        assert_eq!(ein, vec![(0, HotkeyEvent::Toggled(true))]);
        let aus = run(&mut e, &[press("ctrl"), press("alt"), press("b")]);
        assert_eq!(aus, vec![(0, HotkeyEvent::Toggled(false))]);
        // Wie in Python: Bei gehaltenen Modifiern löst erneutes Tippen von B
        // nicht aus, weil der Zustand erst beim nächsten Tastendruck ohne
        // vollständige Kombi zurückgesetzt wird.
        assert!(run(&mut e, &[release("b"), press("b")]).is_empty());
    }

    #[test]
    fn mehrere_kombinationen_unabhaengig() {
        let mut e = engine(&[("ctrl+alt+w", Mode::Hold), ("ctrl+alt+k", Mode::Toggle)]);
        let out = run(&mut e, &[press("ctrl"), press("alt"), press("k"), release("k"), press("w")]);
        assert_eq!(out, vec![(1, HotkeyEvent::Toggled(true)), (0, HotkeyEvent::Pressed)]);
    }

    #[test]
    fn zusaetzliche_modifier_stoeren_nicht() {
        let mut e = engine(&[("ctrl+w", Mode::Hold)]);
        let out = run(&mut e, &[press("shift"), press("ctrl"), press("w")]);
        assert_eq!(out, vec![(0, HotkeyEvent::Pressed)]);
    }

    /// Echter Test mit dem Windows-Tastatur-Hook: schickt künstliche
    /// Tastendrücke Strg+Alt+Umschalt+F23 (von keinem Programm belegt).
    #[cfg(windows)]
    #[test]
    #[ignore = "erzeugt echte Tastendrücke; manuell mit --ignored starten"]
    fn windows_hook_live() {
        use std::time::Duration;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        };

        fn send(vk: u16, up: bool) {
            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
            std::thread::sleep(Duration::from_millis(30));
        }

        let (tx, rx) = mpsc::channel();
        let mut svc = HotkeyService::new();
        let tx2 = tx.clone();
        svc.register("ctrl+alt+shift+f23", Mode::Hold, move |ev| tx.send(("halten", ev)).unwrap());
        svc.register("ctrl+alt+shift+f22", Mode::Toggle, move |ev| tx2.send(("umschalten", ev)).unwrap());
        svc.start().unwrap();

        const CTRL: u16 = 0xA2;
        const ALT: u16 = 0xA4;
        const SHIFT: u16 = 0xA0;
        const F22: u16 = 0x85;
        const F23: u16 = 0x86;
        // Halten: Strg zuerst loslassen muss stoppen
        for vk in [CTRL, ALT, SHIFT, F23] {
            send(vk, false);
        }
        send(CTRL, true);
        for vk in [F23, ALT, SHIFT] {
            send(vk, true);
        }
        // Umschalten zweimal
        for _ in 0..2 {
            for vk in [CTRL, ALT, SHIFT, F22] {
                send(vk, false);
            }
            for vk in [F22, SHIFT, ALT, CTRL] {
                send(vk, true);
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        svc.stop();
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            got,
            vec![
                ("halten", HotkeyEvent::Pressed),
                ("halten", HotkeyEvent::Released),
                ("umschalten", HotkeyEvent::Toggled(true)),
                ("umschalten", HotkeyEvent::Toggled(false)),
            ]
        );
    }
}
