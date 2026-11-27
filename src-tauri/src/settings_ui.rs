//! Befehle für das Einstellungsfenster – Gegenstück zu
//! `BlitzVox alt/blitzvox/ui/settings_window.py` (Daten, Speichern, Keys,
//! Korrekturliste, Modelllisten, Mikrofone).

use std::collections::HashMap;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use zeroize::Zeroizing;

use crate::app::{AppState, VERSION, lock, setup_hotkeys};
use crate::secrets::{ALL_KEYS, delete_api_key, get_api_key, set_api_key};
use crate::settings::{AppSettings, llm_provider, masked, save_settings, stt_provider};
use crate::{autostart, backup, corrections, llm, paste, recorder, stt};

/// Sprachen der Auswahl (wie LANGUAGES in setup_wizard.py).
const LANGUAGES: [(&str, &str); 10] = [
    ("Deutsch", "de"),
    ("Englisch", "en"),
    ("Französisch", "fr"),
    ("Spanisch", "es"),
    ("Italienisch", "it"),
    ("Niederländisch", "nl"),
    ("Polnisch", "pl"),
    ("Portugiesisch", "pt"),
    ("Türkisch", "tr"),
    ("Automatisch erkennen", ""),
];

/// Modellgrößen der lokalen Erkennung (wie _LOCAL_SIZES in setup_wizard.py).
const LOCAL_SIZES: [(&str, &str); 4] = [
    ("Sehr klein (tiny) – am schnellsten, ungenau", "tiny"),
    ("Klein (base) – Standard", "base"),
    ("Mittel (small) – genauer, langsamer", "small"),
    ("Groß (medium) – sehr genau, deutlich langsamer", "medium"),
];

pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.destroy();
    }
    let win = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title(format!("BlitzVox – Einstellungen (v{VERSION})"))
        .inner_size(660.0, 760.0)
        .min_inner_size(620.0, 720.0)
        .maximizable(false)
        .minimizable(false)
        .center()
        .build();
    match win {
        Ok(w) => {
            crate::app::focus_window(&w);
        }
        Err(e) => log::error!("Einstellungen konnten nicht geöffnet werden: {e}"),
    }
}

/// Feste Listen und Texte für den Aufbau des Fensters.
#[tauri::command]
pub fn settings_meta() -> Value {
    let stt: Vec<_> = stt_provider::ALL.iter().map(|id| json!([id, stt_provider::display(id)])).collect();
    let llm: Vec<_> = llm_provider::ALL.iter().map(|id| json!([id, llm_provider::display(id)])).collect();
    let presets: serde_json::Map<String, Value> = llm_provider::ALL
        .iter()
        .map(|id| (id.to_string(), json!(llm_provider::model_presets(id))))
        .collect();
    json!({
        "version": VERSION,
        "windows": cfg!(windows),
        "sttProviders": stt,
        "llmProviders": llm,
        "llmPresets": presets,
        "languages": LANGUAGES,
        "localSizes": LOCAL_SIZES,
        "defaults": AppSettings::default(),
    })
}

/// Aktuelle Einstellungen (frisch von der Platte, wie beim Öffnen in 1.0.8
/// über die zuletzt geladenen Einstellungen der Tray-Klasse).
#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> AppSettings {
    lock(&state.settings).clone()
}

/// Gespeicherte Keys maskiert (`null` = nicht gesetzt).
//
// Alle Befehle mit Schlüsselbund-Zugriff laufen im Hintergrund: Synchrone
// Befehle führt Tauri im Haupt-Thread aus, und der Schlüsselbund kann dauern
// oder auf das Entsperren warten – das Fenster bliebe so lange weiß
// (beobachtet unter Linux Mint, 25.11.2026).
#[tauri::command]
pub async fn keys_status() -> HashMap<String, Option<String>> {
    tauri::async_runtime::spawn_blocking(|| {
        ALL_KEYS
            .iter()
            .map(|k| (k.to_string(), get_api_key(k).map(|v| masked(Some(&v)))))
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Vollständiger Key für das Anzeigefenster.
#[tauri::command]
pub async fn key_get(name: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        ALL_KEYS.contains(&name.as_str()).then(|| get_api_key(&name)).flatten()
    })
    .await
    .ok()
    .flatten()
}

/// Knopf „Löschen“: Key sofort aus dem Schlüsselbund entfernen.
#[tauri::command]
pub async fn key_clear(name: String) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if ALL_KEYS.contains(&name.as_str()) {
            delete_api_key(&name);
        }
    })
    .await;
}

/// Knopf „Einfügen“: Text aus der Zwischenablage.
#[tauri::command]
pub fn clipboard_read() -> String {
    paste::clipboard_text().unwrap_or_default().trim().to_string()
}

#[tauri::command]
pub async fn openrouter_models() -> Vec<String> {
    llm::fetch_openrouter_models().await
}

#[tauri::command]
pub async fn openrouter_stt_models() -> Vec<String> {
    stt::fetch_openrouter_transcription_models().await
}

/// Aufnahmegeräte für die Auswahl.
#[tauri::command]
pub async fn input_devices() -> Vec<Value> {
    tauri::async_runtime::spawn_blocking(|| {
        recorder::list_input_devices()
            .into_iter()
            .map(|d| json!({ "id": d.id, "name": d.name, "default": d.default }))
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// „Speichern“: Keys, Einstellungen und Autostart übernehmen, Tastenkürzel
/// neu einrichten. Ging der Autostart nicht, erscheint eine Warnung.
#[tauri::command]
pub async fn settings_save(
    settings: AppSettings,
    keys: HashMap<String, String>,
    app: AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        for (name, value) in &keys {
            let value = value.trim();
            if ALL_KEYS.contains(&name.as_str()) && !value.is_empty() {
                set_api_key(name, value)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Err(e) = autostart::set_enabled(settings.launch_at_login) {
        app.dialog()
            .message(format!("Der Autostart-Eintrag konnte nicht geändert werden:\n{e}"))
            .title("BlitzVox")
            .kind(MessageDialogKind::Warning)
            .show(|_| {});
    }
    save_settings(&settings).map_err(|e| e.to_string())?;
    *lock(&app.state::<AppState>().settings) = settings;
    setup_hotkeys(&app);
    Ok(())
}

// ── Sicherung (Tab „Erweitert“) ──────────────────────────────────────────────

/// „Sicherung erstellen …“: Speicherort wählen, gespeicherten Stand
/// verschlüsselt schreiben. `None`, wenn der Dialog abgebrochen wurde.
#[tauri::command]
pub async fn backup_create(password: String, app: AppHandle) -> Result<Option<String>, String> {
    let password = Zeroizing::new(password);
    if password.chars().count() < backup::MIN_PASSWORD_CHARS {
        return Err(backup::BackupError::PasswordTooShort.into());
    }
    // Dialog und Argon2 (256 MiB) blockieren: nicht im Haupt-Thread
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let mut dlg = app
            .dialog()
            .file()
            .set_title("Sicherung speichern")
            .add_filter("BlitzVox-Sicherung", &[backup::EXTENSION])
            .set_file_name(backup::default_file_name());
        if let Some(w) = app.get_webview_window("settings") {
            dlg = dlg.set_parent(&w);
        }
        let Some(file) = dlg.blocking_save_file() else { return Ok(None) };
        let path = backup::with_extension(file.into_path().map_err(|e| e.to_string())?);
        backup::create(&path, &password)?;
        Ok(Some(path.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// „Sicherung einspielen …“: Datei wählen, entschlüsseln, nachfragen und
/// dann Einstellungen, Korrekturen und Keys schreiben. `false`, wenn
/// abgebrochen wurde. Bei falschem Passwort wird nichts geändert.
#[tauri::command]
pub async fn backup_restore(password: String, app: AppHandle) -> Result<bool, String> {
    let password = Zeroizing::new(password);
    if password.is_empty() {
        return Err("Bitte das Passwort der Sicherung eingeben.".into());
    }
    let handle = app.clone();
    let restored = tauri::async_runtime::spawn_blocking(move || -> Result<Option<AppSettings>, String> {
        let win = handle.get_webview_window("settings");
        let mut dlg = handle
            .dialog()
            .file()
            .set_title("Sicherung einspielen")
            .add_filter("BlitzVox-Sicherung", &[backup::EXTENSION]);
        if let Some(w) = &win {
            dlg = dlg.set_parent(w);
        }
        let Some(file) = dlg.blocking_pick_file() else { return Ok(None) };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let content = backup::read_from(&path, &password)?;
        let mut ask = handle
            .dialog()
            .message(format!(
                "{}\n\nDie aktuellen Einstellungen und Korrekturen werden ersetzt. \
Keys aus der Sicherung überschreiben die gespeicherten, alle anderen Keys \
bleiben erhalten. Nicht gespeicherte Änderungen in diesem Fenster gehen \
verloren.\n\nJetzt einspielen?",
                content.summary()
            ))
            .title("BlitzVox – Sicherung einspielen")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Einspielen".into(), "Abbrechen".into()));
        if let Some(w) = &win {
            ask = ask.parent(w);
        }
        if !ask.blocking_show() {
            return Ok(None);
        }
        backup::apply(&content).map(Some).map_err(String::from)
    })
    .await
    .map_err(|e| e.to_string())??;
    let Some(settings) = restored else { return Ok(false) };
    if let Err(e) = autostart::set_enabled(settings.launch_at_login) {
        app.dialog()
            .message(format!("Der Autostart-Eintrag konnte nicht geändert werden:\n{e}"))
            .title("BlitzVox")
            .kind(MessageDialogKind::Warning)
            .show(|_| {});
    }
    *lock(&app.state::<AppState>().settings) = settings;
    setup_hotkeys(&app);
    Ok(true)
}

// ── Gelernte Korrekturen (Tab „Korrekturen“, sofort gespeichert) ─────────────

/// Liste (falsch, richtig), sortiert nach „falsch“ ohne Groß/Klein.
#[tauri::command]
pub fn corr_list() -> Vec<(String, String)> {
    let data = corrections::load();
    let mut rows: Vec<(String, String)> = data
        .replacements
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
        .collect();
    rows.sort_by_key(|(w, _)| w.to_lowercase());
    rows
}

/// „Hinzufügen“. `true`, wenn gespeichert wurde (dann Felder leeren).
#[tauri::command]
pub fn corr_add(wrong: String, right: String, app: AppHandle) -> Result<bool, String> {
    let (wrong, right) = (wrong.trim(), right.trim());
    if wrong.is_empty() || right.is_empty() {
        return Ok(false);
    }
    if wrong.to_lowercase() == right.to_lowercase() {
        app.dialog()
            .message(
                "Falsch und richtig unterscheiden sich nur in der Groß-/Kleinschreibung. \
Das regelt das Sprachmodell, dafür ist kein Eintrag nötig.",
            )
            .title("BlitzVox")
            .show(|_| {});
        return Ok(false);
    }
    corrections::add_replacement(wrong, right).map_err(|e| e.to_string())?;
    Ok(true)
}

/// „Ausgewählte löschen“.
#[tauri::command]
pub fn corr_remove(wrongs: Vec<String>) -> Result<(), String> {
    for w in wrongs {
        corrections::remove_replacement(&w).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Knopf „Einrichtungsassistent starten“: Einstellungen schließen, Assistent öffnen.
/// Muss `async` sein: Ein Fenster in einem synchronen Befehl zu erzeugen,
/// blockiert unter Windows den Hauptthread (Assistent und „Beenden“ hingen).
#[tauri::command]
pub async fn request_setup_wizard(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.close();
    }
    crate::app::open_setup_wizard(&app, false);
}
