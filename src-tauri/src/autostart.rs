//! Autostart – 1:1-Nachbau von `BlitzVox alt/blitzvox/services/autostart.py`,
//! damit vorhandene Einträge von 1.0.8 erkannt und ersetzt werden.
//!
//! Windows: Wert `BlitzVox` unter
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (keine Adminrechte).
//! Linux: Desktop-Datei `~/.config/autostart/blitzvox.desktop` (XDG).
//! Andere Systeme: keine Funktion.

#![allow(dead_code)]

/// Befehl für den Eintrag: Pfad der laufenden Programmdatei in Anführungszeichen.
fn exe_path() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "BlitzVox";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: u32) -> Result<HKEY, String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: gültige Zeichenkette und Ausgabezeiger.
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, access, &mut key) };
        if rc != ERROR_SUCCESS {
            return Err(format!("Registry-Schlüssel nicht zu öffnen (Fehler {rc})"));
        }
        Ok(key)
    }

    pub fn is_enabled() -> bool {
        let Ok(key) = open(KEY_QUERY_VALUE) else { return false };
        // SAFETY: Abfrage, ob der Wert existiert (ohne Daten zu lesen).
        let rc = unsafe {
            RegQueryValueExW(
                key,
                wide(VALUE_NAME).as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        unsafe { RegCloseKey(key) };
        rc == ERROR_SUCCESS
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let key = open(KEY_SET_VALUE)?;
        let result = if enabled {
            let cmd = format!("\"{}\"", super::exe_path()?);
            let data = wide(&cmd);
            // SAFETY: Daten als UTF-16 mit abschließender Null, Länge in Bytes.
            let rc = unsafe {
                RegSetValueExW(
                    key,
                    wide(VALUE_NAME).as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr() as *const u8,
                    (data.len() * 2) as u32,
                )
            };
            if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!("Autostart-Eintrag nicht gesetzt (Fehler {rc})")) }
        } else {
            // SAFETY: Löschen des Werts; fehlt er, ist das kein Fehler.
            let rc = unsafe { RegDeleteValueW(key, wide(VALUE_NAME).as_ptr()) };
            if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(format!("Autostart-Eintrag nicht entfernt (Fehler {rc})"))
            }
        };
        unsafe { RegCloseKey(key) };
        result
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    fn desktop_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
        base.join("autostart").join("blitzvox.desktop")
    }

    /// Desktop-Entry-Exec: Argument in Anführungszeichen, Sonderzeichen escapen.
    fn quote(arg: &str) -> String {
        let mut s = arg.to_string();
        for ch in ['\\', '"', '`', '$'] {
            s = s.replace(ch, &format!("\\{ch}"));
        }
        format!("\"{s}\"")
    }

    pub fn is_enabled() -> bool {
        desktop_path().exists()
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let path = desktop_path();
        if enabled {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let content = format!(
                "[Desktop Entry]\nType=Application\nName=BlitzVox\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
                quote(&super::exe_path()?)
            );
            std::fs::write(&path, content).map_err(|e| e.to_string())
        } else {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn is_enabled() -> bool {
        false
    }
    pub fn set_enabled(_enabled: bool) -> Result<(), String> {
        Ok(())
    }
}

/// True, wenn der Autostart-Eintrag existiert.
pub fn is_enabled() -> bool {
    platform::is_enabled()
}

/// Setzt oder entfernt den Autostart-Eintrag. Fehler werden zurückgegeben.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    platform::set_enabled(enabled)
}
