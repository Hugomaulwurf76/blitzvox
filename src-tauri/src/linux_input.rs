//! Tastatur-Ein- und -Ausgabe unter Linux, wenn X11 nicht reicht (Wayland).
//!
//! Unter Wayland sieht die X11-Erweiterung RECORD nur Tasten, die ein
//! X11-Programm erreichen. Deshalb gibt es hier zwei weitere Wege:
//!
//! - **Tastatur direkt lesen** (`/dev/input/event*`): funktioniert auf jedem
//!   Desktop, braucht Leserechte auf die Tastatur-Geräte. Das .deb richtet
//!   dafür eine udev-Regel ein (`linux/70-blitzvox.rules`).
//! - **Portal „GlobalShortcuts“** (xdg-desktop-portal): Der Desktop meldet die
//!   Kürzel selbst. Gibt es z. B. unter KDE Plasma 6 und GNOME ab 48.
//!
//! Außerdem eine virtuelle Tastatur über `/dev/uinput`, mit der Strg+V in
//! Wayland-Programme geschickt wird (wie ydotool, aber ohne Hilfsdienst).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use evdev::{Device, EventType, InputEvent, KeyCode};

/// Name der eigenen virtuellen Tastatur – wird beim Lesen übersprungen.
const VIRTUAL_NAME: &str = "BlitzVox virtuelle Tastatur";

/// Läuft eine Wayland-Sitzung?
pub fn session_is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|s| s.eq_ignore_ascii_case("wayland")).unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

// ── Tastatur direkt lesen ────────────────────────────────────────────────────

/// Rückmeldung je Taste: (gedrückt?, Linux-Tastencode)
pub type KeyCallback = Arc<dyn Fn(bool, u16) + Send + Sync>;

fn is_keyboard(dev: &Device) -> bool {
    if dev.name() == Some(VIRTUAL_NAME) {
        return false;
    }
    dev.supported_keys().is_some_and(|k| {
        k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_Z) && k.contains(KeyCode::KEY_ENTER)
    })
}

fn event_nodes() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return Vec::new() };
    dir.filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
        .collect()
}

fn read_device(path: PathBuf, mut dev: Device, on_key: KeyCallback, open: Arc<Mutex<HashSet<PathBuf>>>) {
    let name = format!("blitzvox-evdev-{}", path.display());
    let spawned = thread::Builder::new().name(name).spawn(move || {
        loop {
            let events = match dev.fetch_events() {
                Ok(ev) => ev,
                Err(e) => {
                    log::info!("Tastatur {} nicht mehr lesbar: {e}", path.display());
                    break;
                }
            };
            for ev in events {
                if ev.event_type() == EventType::KEY {
                    // 1 = gedrückt, 2 = Wiederholung (wie gedrückt), 0 = losgelassen
                    on_key(ev.value() != 0, ev.code());
                }
            }
        }
        open.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
    });
    if let Err(e) = spawned {
        log::warn!("Lese-Thread für Tastatur nicht gestartet: {e}");
    }
}

/// Öffnet alle neuen Tastaturen; gibt die Zahl der neu geöffneten zurück.
fn open_new(on_key: &KeyCallback, open: &Arc<Mutex<HashSet<PathBuf>>>) -> usize {
    let mut count = 0;
    for (path, dev) in evdev::enumerate() {
        if !is_keyboard(&dev) {
            continue;
        }
        let mut set = open.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(path.clone()) {
            continue;
        }
        drop(set);
        log::info!("Tastatur gefunden: {} ({})", dev.name().unwrap_or("?"), path.display());
        read_device(path, dev, on_key.clone(), open.clone());
        count += 1;
    }
    count
}

/// Startet das Mitlesen aller Tastaturen. Später angesteckte Tastaturen werden
/// alle 3 s gesucht. Fehler, wenn keine Tastatur lesbar ist (meist fehlende Rechte).
pub fn start_keyboard_reader(on_key: KeyCallback) -> Result<usize, String> {
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
    let count = open_new(&on_key, &open);
    if count == 0 {
        return Err(if event_nodes().is_empty() {
            "Keine Eingabegeräte unter /dev/input gefunden.".into()
        } else {
            "Kein Lesezugriff auf die Tastatur (/dev/input). Die udev-Regel von BlitzVox fehlt \
             oder ist noch nicht aktiv – nach der Installation einmal ab- und wieder anmelden."
                .into()
        });
    }
    thread::Builder::new()
        .name("blitzvox-evdev-scan".into())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(3));
                open_new(&on_key, &open);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(count)
}

/// Tastenname (wie pynput) für einen Linux-Tastencode bei US-Belegung.
/// Nur Rückfall, wenn keine Tastaturbelegung von X11/XWayland bekannt ist.
pub fn us_key_name(code: u16) -> Option<&'static str> {
    const LETTERS: [(u16, &str); 26] = [
        (30, "a"), (48, "b"), (46, "c"), (32, "d"), (18, "e"), (33, "f"), (34, "g"),
        (35, "h"), (23, "i"), (36, "j"), (37, "k"), (38, "l"), (50, "m"), (49, "n"),
        (24, "o"), (25, "p"), (16, "q"), (19, "r"), (31, "s"), (20, "t"), (22, "u"),
        (47, "v"), (17, "w"), (45, "x"), (21, "y"), (44, "z"),
    ];
    if let Some((_, n)) = LETTERS.iter().find(|(c, _)| *c == code) {
        return Some(n);
    }
    Some(match code {
        29 | 97 => "ctrl",
        42 | 54 => "shift",
        56 | 100 => "alt",
        125 | 126 => "cmd",
        2 => "1", 3 => "2", 4 => "3", 5 => "4", 6 => "5",
        7 => "6", 8 => "7", 9 => "8", 10 => "9", 11 => "0",
        59 => "f1", 60 => "f2", 61 => "f3", 62 => "f4", 63 => "f5", 64 => "f6",
        65 => "f7", 66 => "f8", 67 => "f9", 68 => "f10", 87 => "f11", 88 => "f12",
        183 => "f13", 184 => "f14", 185 => "f15", 186 => "f16", 187 => "f17", 188 => "f18",
        189 => "f19", 190 => "f20", 191 => "f21", 192 => "f22", 193 => "f23", 194 => "f24",
        57 => "space",
        28 | 96 => "enter",
        1 => "esc",
        15 => "tab",
        14 => "backspace",
        111 => "delete",
        110 => "insert",
        102 => "home",
        107 => "end",
        104 => "page_up",
        109 => "page_down",
        103 => "up",
        108 => "down",
        105 => "left",
        106 => "right",
        58 => "caps_lock",
        69 => "num_lock",
        70 => "scroll_lock",
        119 => "pause",
        99 => "print_screen",
        127 => "menu",
        _ => return None,
    })
}

// ── Portal „GlobalShortcuts“ ─────────────────────────────────────────────────

/// Rückmeldung: (Nummer des Kürzels in der zuletzt gebundenen Liste, aktiv?)
pub type PortalCallback = Arc<dyn Fn(usize, bool) + Send + Sync>;

/// Ein gewünschtes Kürzel: Beschreibung für den Systemdialog und Auslöser im
/// Format der XDG-Spezifikation „shortcuts“ (z. B. `CTRL+ALT+w`).
#[derive(Clone, Debug)]
pub struct PortalShortcut {
    pub description: String,
    pub trigger: String,
}

pub struct PortalHandle {
    tx: tokio::sync::mpsc::UnboundedSender<Vec<PortalShortcut>>,
}

impl PortalHandle {
    /// Bindet die Kürzel neu (alte Sitzung wird geschlossen).
    pub fn bind(&self, shortcuts: Vec<PortalShortcut>) {
        let _ = self.tx.send(shortcuts);
    }
}

/// Verbindet sich mit dem Portal. Fehler, wenn der Desktop es nicht anbietet.
pub fn start_portal(on_event: PortalCallback) -> Result<PortalHandle, String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PortalShortcut>>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    thread::Builder::new()
        .name("blitzvox-portal".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            rt.block_on(portal_loop(on_event, rx, ready_tx));
        })
        .map_err(|e| e.to_string())?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Portal antwortet nicht.".to_string())??;
    Ok(PortalHandle { tx })
}

async fn portal_loop(
    on_event: PortalCallback,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<PortalShortcut>>,
    ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
) {
    use ashpd::desktop::CreateSessionOptions;
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;

    let setup = async {
        let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
        if proxy.version() == 0 {
            return Err("Der Desktop bietet keine globalen Tastenkürzel über das Portal an.".to_string());
        }
        let activated = proxy.receive_activated().await.map_err(|e| e.to_string())?;
        let deactivated = proxy.receive_deactivated().await.map_err(|e| e.to_string())?;
        Ok((proxy, activated, deactivated))
    };
    let (proxy, activated, deactivated) = match setup.await {
        Ok(v) => {
            let _ = ready_tx.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    let mut activated = std::pin::pin!(activated);
    let mut deactivated = std::pin::pin!(deactivated);
    // Nur eine Sitzung ist offen (die alte wird vor dem Neubinden geschlossen),
    // daher gehören eintreffende Signale zur aktuellen Sitzung.
    let mut session = None;

    loop {
        tokio::select! {
            wanted = rx.recv() => {
                let Some(wanted) = wanted else { break };
                if let Some(old) = session.take() {
                    let old: ashpd::desktop::Session<GlobalShortcuts> = old;
                    let _ = old.close().await;
                }
                if wanted.is_empty() {
                    continue;
                }
                let new_session = match proxy.create_session(CreateSessionOptions::default()).await {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("Portal: Sitzung nicht angelegt: {e}");
                        continue;
                    }
                };
                let list: Vec<NewShortcut> = wanted
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        NewShortcut::new(format!("bv{i}"), s.description.clone())
                            .preferred_trigger(Some(s.trigger.as_str()))
                    })
                    .collect();
                match proxy.bind_shortcuts(&new_session, &list, None, BindShortcutsOptions::default()).await {
                    Ok(req) => match req.response() {
                        Ok(bound) => {
                            for s in bound.shortcuts() {
                                log::info!("Portal-Kürzel {}: {}", s.id(), s.trigger_description());
                            }
                        }
                        Err(e) => log::warn!("Portal: Kürzel nicht bestätigt: {e}"),
                    },
                    Err(e) => log::warn!("Portal: Kürzel nicht gebunden: {e}"),
                }
                session = Some(new_session);
            }
            Some(ev) = activated.next() => {
                if session.is_some() {
                    if let Some(i) = shortcut_index(ev.shortcut_id()) {
                        on_event(i, true);
                    }
                }
            }
            Some(ev) = deactivated.next() => {
                if session.is_some() {
                    if let Some(i) = shortcut_index(ev.shortcut_id()) {
                        on_event(i, false);
                    }
                }
            }
        }
    }
}

fn shortcut_index(id: &str) -> Option<usize> {
    id.strip_prefix("bv")?.parse().ok()
}

// ── Virtuelle Tastatur (Strg+V) ──────────────────────────────────────────────

static VIRTUAL: Mutex<Option<evdev::uinput::VirtualDevice>> = Mutex::new(None);

fn key(code: KeyCode, down: bool) -> InputEvent {
    InputEvent::new(EventType::KEY.0, code.0, if down { 1 } else { 0 })
}

/// Schickt Strg+V über eine virtuelle Tastatur (`/dev/uinput`).
pub fn paste_ctrl_v() -> Result<(), String> {
    let mut guard = VIRTUAL.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let mut keys = evdev::AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::KEY_LEFTCTRL);
        keys.insert(KeyCode::KEY_V);
        let dev = evdev::uinput::VirtualDevice::builder()
            .and_then(|b| b.name(VIRTUAL_NAME).with_keys(&keys))
            .and_then(|b| b.build())
            .map_err(|e| format!("Virtuelle Tastatur nicht verfügbar (/dev/uinput): {e}"))?;
        *guard = Some(dev);
        // Der Desktop braucht einen Moment, bis er das neue Gerät kennt.
        thread::sleep(Duration::from_millis(400));
    }
    let dev = guard.as_mut().expect("eben angelegt");
    let steps = [
        key(KeyCode::KEY_LEFTCTRL, true),
        key(KeyCode::KEY_V, true),
        key(KeyCode::KEY_V, false),
        key(KeyCode::KEY_LEFTCTRL, false),
    ];
    for ev in steps {
        dev.emit(&[ev]).map_err(|e| e.to_string())?;
        thread::sleep(Duration::from_millis(15));
    }
    Ok(())
}
