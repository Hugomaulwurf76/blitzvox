//! Zwischenablage und automatisches Einfügen – Nachbau von
//! `BlitzVox alt/blitzvox/services/paste.py`.
//!
//! Windows: Das BlitzVox-Fenster wird vor dem Einfügen versteckt, Windows
//! gibt den Fokus dabei ans vorherige Fenster zurück; danach wird Strg+V als
//! echte Eingabe gesendet (`keybd_event`). Das Fenster merkt sich 1.0.8 zwar
//! (`save_focus`), stellt den Fokus aber nicht selbst wieder her – hier genauso.
//!
//! Linux: X11 mit `xdotool` (vorheriges Fenster aktivieren, dann Strg+V),
//! Wayland mit `wtype`, dann `ydotool`, zuletzt `xdotool`.
//!
//! Die Zwischenablage nutzt `arboard` – dieselbe Bibliothek, die auch das
//! offizielle Tauri-Plugin „clipboard-manager“ verwendet. Unter Linux gehört
//! der Inhalt dem Programm, solange es läuft; deshalb bleibt die
//! Zwischenablage hier dauerhaft geöffnet (wie bei Qt in 1.0.8).

// Wird erst mit den Workflows und der Oberfläche vollständig benutzt.
#![allow(dead_code)]

use std::sync::Mutex;

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Text in die Zwischenablage legen.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    let cb = guard.as_mut().expect("Zwischenablage geöffnet");
    cb.set_text(text).map_err(|e| e.to_string())
}

/// Zum Testen: aktuellen Text der Zwischenablage lesen.
pub fn clipboard_text() -> Result<String, String> {
    let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    guard.as_mut().expect("Zwischenablage geöffnet").get_text().map_err(|e| e.to_string())
}

/// Vor dem Öffnen des BlitzVox-Fensters aufrufen – merkt sich das aktive Fenster.
pub fn save_focus() {
    platform::save_focus();
}

/// Text in die Zwischenablage legen und mit Strg+V ins fokussierte Fenster
/// einfügen. `false`, wenn das Einfügen nicht möglich war (der Text liegt
/// dann trotzdem in der Zwischenablage, sofern die Zwischenablage ging).
pub fn paste_to_focused_window(text: &str) -> bool {
    if let Err(e) = copy_to_clipboard(text) {
        log::warn!("Zwischenablage: {e}");
    }
    platform::paste()
}

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::thread::sleep;
    use std::time::Duration;

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, keybd_event};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    /// Gemerktes Fenster (wird wie in 1.0.8 nicht zum Zurückholen genutzt).
    static SAVED_HWND: AtomicIsize = AtomicIsize::new(0);

    pub(super) fn save_focus() {
        // SAFETY: reine Abfrage ohne Parameter.
        let hwnd = unsafe { GetForegroundWindow() };
        SAVED_HWND.store(hwnd as isize, Ordering::Relaxed);
    }

    pub(super) fn paste() -> bool {
        const VK_CONTROL: u8 = 0x11;
        const VK_V: u8 = 0x56;
        // SAFETY: keybd_event erzeugt nur Tastatur-Ereignisse.
        unsafe {
            keybd_event(VK_CONTROL, 0, 0, 0);
            keybd_event(VK_V, 0, 0, 0);
            sleep(Duration::from_millis(20));
            keybd_event(VK_V, 0, KEYEVENTF_KEYUP, 0);
            keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, 0);
        }
        true
    }
}

#[cfg(not(windows))]
mod platform {
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    /// Gemerkte X11-Fenster-ID (für `xdotool windowactivate`).
    static PREV_WIN: Mutex<Option<String>> = Mutex::new(None);

    pub(super) fn save_focus() {
        if let Some((0, out)) = run_output(&["xdotool", "getactivewindow"], Duration::from_secs(2)) {
            let id = out.trim().to_string();
            if !id.is_empty() {
                *PREV_WIN.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
            }
        }
    }

    pub(super) fn paste() -> bool {
        let wayland = std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase() == "wayland";
        if wayland {
            // 1. wtype – KDE, Sway, Hyprland und alle wlroots-basierten
            if try_run(&["wtype", "-M", "ctrl", "v", "-m", "ctrl"]) {
                return true;
            }
            // 2. wtype mit anderer Syntax (ältere Versionen)
            if try_run(&["wtype", "-M", "ctrl", "-P", "v", "-p", "v", "-m", "ctrl"]) {
                return true;
            }
            // 3. eigene virtuelle Tastatur über /dev/uinput – alle Desktops
            //    (neu in 2.0; Zugriff über die udev-Regel aus dem .deb)
            #[cfg(target_os = "linux")]
            match crate::linux_input::paste_ctrl_v() {
                Ok(()) => return true,
                Err(e) => log::warn!("Einfügen: {e}"),
            }
            // 4. ydotool – GNOME unter Wayland
            if try_run(&["ydotool", "key", "29:1", "47:1", "47:0", "29:0"]) {
                return true;
            }
            // 5. xdotool als letzter Versuch (klappt unter XWayland für X11-Programme)
            paste_via_xdotool()
        } else {
            // X11 – vorrangig xdotool, sonst wtype falls vorhanden
            paste_via_xdotool() || try_run(&["wtype", "-M", "ctrl", "v", "-m", "ctrl"])
        }
    }

    fn paste_via_xdotool() -> bool {
        let prev = PREV_WIN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(id) = prev {
            // Ergebnis egal, wie in 1.0.8; fehlt xdotool, scheitert gleich der nächste Aufruf
            if run_output(&["xdotool", "windowactivate", "--sync", &id], Duration::from_secs(3)).is_none() {
                return false;
            }
            sleep(Duration::from_millis(100));
        }
        try_run(&["xdotool", "key", "--clearmodifiers", "ctrl+v"])
    }

    fn try_run(cmd: &[&str]) -> bool {
        matches!(run_output(cmd, Duration::from_secs(3)), Some((0, _)))
    }

    /// Startet ein Programm mit Zeitlimit. `None`, wenn es fehlt oder zu
    /// lange braucht (wie FileNotFoundError/TimeoutExpired in Python).
    fn run_output(cmd: &[&str], timeout: Duration) -> Option<(i32, String)> {
        let mut child = Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut out = String::new();
                    if let Some(mut so) = child.stdout.take() {
                        use std::io::Read;
                        let _ = so.read_to_string(&mut out);
                    }
                    return Some((status.code().unwrap_or(-1), out));
                }
                Ok(None) if start.elapsed() < timeout => sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "überschreibt die echte Zwischenablage; manuell mit --ignored starten"]
    fn zwischenablage_rundlauf() {
        let alt = clipboard_text().ok();
        copy_to_clipboard("BlitzVox-Test äöü „Zitat“ 😀").unwrap();
        assert_eq!(clipboard_text().unwrap(), "BlitzVox-Test äöü „Zitat“ 😀");
        if let Some(a) = alt {
            let _ = copy_to_clipboard(&a);
        }
    }
}
