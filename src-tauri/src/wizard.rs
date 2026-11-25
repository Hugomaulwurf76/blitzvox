//! Einrichtungsassistent – Rust-Seite zu `ui/wizard.html`, Nachbau von
//! `BlitzVox alt/blitzvox/ui/setup_wizard.py`.
//!
//! Der Assistent arbeitet auf einer Kopie der Einstellungen (in der
//! Oberfläche) und übergibt sie erst bei „Fertig“. API-Keys werden schon beim
//! Testen bzw. Weiterklicken gespeichert, weil das Probediktat sie braucht.

use std::sync::Arc;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::app::{AppState, VERSION, lock, setup_hotkeys, stop_hotkeys};
use crate::hotkeys::{HotkeyEvent, HotkeyService, Mode};
use crate::recorder::AudioRecorder;
use crate::secrets::{KEY_ANTHROPIC, KEY_ASSEMBLYAI, KEY_GROQ, KEY_OPENAI, KEY_OPENROUTER, get_api_key, set_api_key};
use crate::settings::{AppSettings, save_settings, stt_provider};
use crate::workflow::{Phase, WorkflowKind, WorkflowRunner};
use crate::{autostart, http};

/// Laufende Dinge des Assistenten (Pegelanzeige, Probediktat, Tastenkürzel).
#[derive(Default)]
pub struct WizardState {
    automatic: bool,
    finished: bool,
    monitor: Option<AudioRecorder>,
    runner: Option<WorkflowRunner>,
    hotkeys: Option<HotkeyService>,
}

/// STT-Anbieter, die zwingend einen API-Key brauchen.
fn stt_key_required(provider: &str) -> Option<&'static str> {
    match provider {
        stt_provider::GROQ => Some(KEY_GROQ),
        stt_provider::OPENAI => Some(KEY_OPENAI),
        stt_provider::ASSEMBLYAI => Some(KEY_ASSEMBLYAI),
        stt_provider::OPENROUTER => Some(KEY_OPENROUTER),
        _ => None,
    }
}

/// Soll der Assistent beim Start automatisch erscheinen? Beim allerersten
/// Start oder wenn die Spracherkennung nicht einsatzbereit ist.
pub fn needs_setup(s: &AppSettings) -> bool {
    if !s.has_seen_onboarding {
        return true;
    }
    let provider = s.transcription_model.stt_provider.as_str();
    if let Some(key) = stt_key_required(provider) {
        return get_api_key(key).is_none();
    }
    if provider == stt_provider::CUSTOM {
        let url = if s.transcription_model.custom_stt_url.is_empty() {
            &s.custom_stt_url
        } else {
            &s.transcription_model.custom_stt_url
        };
        return url.trim().is_empty();
    }
    false
}

fn wizard_state(app: &AppHandle) -> std::sync::MutexGuard<'_, WizardState> {
    lock(&app.state::<AppState>().inner().wizard)
}

/// Öffnet den Assistenten. `automatic`: beim Programmstart, sonst über den
/// Knopf im Tab „Erweitert“.
pub fn open(app: &AppHandle, automatic: bool) {
    if let Some(w) = app.get_webview_window("wizard") {
        crate::app::focus_window(&w);
        return;
    }
    // Eigene Tastenkürzel pausieren: Das Probediktat nutzt das Kürzel selbst
    stop_hotkeys(app);
    *wizard_state(app) = WizardState { automatic, ..Default::default() };

    let win = WebviewWindowBuilder::new(app, "wizard", WebviewUrl::App("wizard.html".into()))
        .title(format!("BlitzVox – Einrichtung (v{VERSION})"))
        .inner_size(700.0, 660.0)
        .min_inner_size(660.0, 580.0)
        .maximizable(false)
        .minimizable(false)
        .center()
        .build();
    match win {
        Ok(w) => {
            let handle = app.clone();
            w.on_window_event(move |ev| {
                if let tauri::WindowEvent::Destroyed = ev {
                    on_closed(&handle);
                }
            });
            crate::app::focus_window(&w);
        }
        Err(e) => {
            log::error!("Einrichtungsassistent konnte nicht geöffnet werden: {e}");
            setup_hotkeys(app);
        }
    }
}

fn cleanup(ws: &mut WizardState) {
    if let Some(mut m) = ws.monitor.take() {
        m.discard();
    }
    if let Some(mut r) = ws.runner.take() {
        r.cancel();
    }
    if let Some(mut h) = ws.hotkeys.take() {
        h.stop();
    }
}

/// Fenster geschlossen: bei „Fertig“ ist alles erledigt, sonst wie Abbrechen
/// (Eingaben verwerfen; beim automatischen Start den ersten Start trotzdem als
/// erledigt merken). Danach Tastenkürzel wieder einrichten.
fn on_closed(app: &AppHandle) {
    let (finished, automatic) = {
        let mut ws = wizard_state(app);
        cleanup(&mut ws);
        (ws.finished, ws.automatic)
    };
    if !finished && automatic {
        let state = app.state::<AppState>();
        let mut s = lock(&state.settings);
        if !s.has_seen_onboarding {
            s.has_seen_onboarding = true;
            let _ = save_settings(&s);
        }
        drop(s);
        notify(
            app,
            "Einrichtung abgebrochen",
            "Du kannst sie jederzeit unter Einstellungen → Erweitert erneut starten.",
        );
    }
    setup_hotkeys(app);
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

// ── Befehle ──────────────────────────────────────────────────────────────────

/// Namen der Grafikkarten, die whisper.cpp über Vulkan nutzen kann.
///
/// Bewusst über die Backend-Registry von ggml und nicht über
/// `whisper_rs::vulkan::list_devices()`: Letztere ruft die Vulkan-Initialisierung
/// ohne Fehlerabfang auf. Gibt es einen Vulkan-Loader, aber keinen
/// Vulkan-Grafiktreiber (z. B. VM), wirft sie einen C++-Fehler, und das Programm
/// bricht hart ab (beobachtet in einer Proxmox-VM, 25.11.2026). Die Registry
/// fängt diesen Fehler ab und meldet dann einfach kein Vulkan-Gerät.
fn vulkan_gpu_names() -> Vec<String> {
    use std::ffi::CStr;
    use whisper_rs_sys::{
        ggml_backend_dev_description, ggml_backend_reg_by_name, ggml_backend_reg_dev_count,
        ggml_backend_reg_dev_get,
    };
    // SAFETY: Reine Abfragen der ggml-Registry; Zeiger werden vor Gebrauch geprüft,
    // die Texte gehören ggml und werden sofort kopiert.
    unsafe {
        let reg = ggml_backend_reg_by_name(c"Vulkan".as_ptr());
        if reg.is_null() {
            return Vec::new();
        }
        (0..ggml_backend_reg_dev_count(reg))
            .filter_map(|i| {
                let dev = ggml_backend_reg_dev_get(reg, i);
                if dev.is_null() {
                    return None;
                }
                let desc = ggml_backend_dev_description(dev);
                (!desc.is_null()).then(|| CStr::from_ptr(desc).to_string_lossy().into_owned())
            })
            .collect()
    }
}

/// Startdaten: Kopie der Einstellungen, feste Listen und Systeminfos.
#[tauri::command]
pub async fn wizard_init(app: AppHandle) -> Value {
    let settings = lock(&app.state::<AppState>().settings).clone();
    // Grafikkarten, die whisper.cpp über Vulkan sieht
    let gpus: Vec<String> = tauri::async_runtime::spawn_blocking(|| {
        if cfg!(debug_assertions) && std::env::var("BLITZVOX_TEST_NO_GPU").is_ok() {
            return Vec::new();
        }
        vulkan_gpu_names()
    })
    .await
    .unwrap_or_default();
    let wayland = cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase() == "wayland";
    let xdotool_missing = cfg!(target_os = "linux") && !wayland && !in_path("xdotool");
    // Nur für Tests im Entwicklungsbuild: Assistent auf einer bestimmten Seite öffnen
    let start_page: usize = if cfg!(debug_assertions) {
        std::env::var("BLITZVOX_TEST_WIZARD_PAGE").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    } else {
        0
    };
    json!({
        "startPage": start_page,
        "settings": settings,
        "meta": crate::settings_ui::settings_meta(),
        "wayland": wayland,
        "xdotoolMissing": xdotool_missing,
        "gpus": gpus,
    })
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

/// Prüft einen API-Key mit einer reinen Leseanfrage beim Anbieter.
/// Liefert [Status, Meldung] mit Status "ok", "invalid" oder "unreachable".
#[tauri::command]
pub async fn key_check(name: String, key: String) -> (String, String) {
    let (url, headers): (&str, Vec<(&str, String)>) = match name.as_str() {
        KEY_GROQ => ("https://api.groq.com/openai/v1/models", vec![("Authorization", format!("Bearer {key}"))]),
        KEY_OPENAI => ("https://api.openai.com/v1/models", vec![("Authorization", format!("Bearer {key}"))]),
        KEY_OPENROUTER => ("https://openrouter.ai/api/v1/key", vec![("Authorization", format!("Bearer {key}"))]),
        KEY_ASSEMBLYAI => ("https://api.assemblyai.com/v2/transcript?limit=1", vec![("authorization", key.clone())]),
        KEY_ANTHROPIC => (
            "https://api.anthropic.com/v1/models",
            vec![("x-api-key", key.clone()), ("anthropic-version", "2023-06-01".into())],
        ),
        _ => return ("unreachable".into(), "Für diesen Anbieter gibt es keinen Key-Test.".into()),
    };
    let Ok(client) = http::client(10) else {
        return ("unreachable".into(), "Anbieter nicht erreichbar.".into());
    };
    let mut req = client.get(url);
    for (k, v) in &headers {
        req = req.header(*k, v);
    }
    match req.send().await {
        Err(e) => ("unreachable".into(), format!("Anbieter nicht erreichbar ({}).", error_kind(&e))),
        Ok(resp) => match resp.status().as_u16() {
            200 => ("ok".into(), "Key ist gültig.".into()),
            401 | 403 => ("invalid".into(), "Der Key wurde abgelehnt. Bitte prüfen und neu einfügen.".into()),
            code => ("unreachable".into(), format!("Unerwartete Antwort vom Anbieter (Fehler {code}).")),
        },
    }
}

fn error_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "Zeitüberschreitung"
    } else if e.is_connect() {
        "keine Verbindung"
    } else {
        "Netzwerkfehler"
    }
}

/// Key im Schlüsselbund speichern.
#[tauri::command]
pub async fn key_store(name: String, key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || key_store_blocking(name, key))
        .await
        .map_err(|e| e.to_string())?
}

fn key_store_blocking(name: String, key: String) -> Result<(), String> {
    set_api_key(&name, key.trim())
}

/// Gespeicherter Key maskiert, `null` wenn keiner da ist.
#[tauri::command]
pub async fn key_masked(name: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || key_masked_blocking(name)).await.ok().flatten()
}

fn key_masked_blocking(name: String) -> Option<String> {
    get_api_key(&name).map(|k| crate::settings::masked(Some(&k)))
}

/// Prüft, ob Ollama läuft. Liefert [Status, Meldung, installierte Modelle].
#[tauri::command]
pub async fn ollama_check(url: String) -> (String, String, Vec<String>) {
    let unreachable = || {
        (
            "invalid".to_string(),
            "Ollama ist unter dieser Adresse nicht erreichbar. Ist Ollama installiert und gestartet?".to_string(),
            Vec::new(),
        )
    };
    let Ok(client) = http::client(5) else { return unreachable() };
    let Ok(resp) = client.get(format!("{}/api/tags", url.trim_end_matches('/'))).send().await else {
        return unreachable();
    };
    if resp.status().as_u16() != 200 {
        return ("invalid".into(), format!("Ollama antwortet mit Fehler {}.", resp.status().as_u16()), Vec::new());
    }
    let mut names: Vec<String> = resp
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v["models"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m["name"].as_str().filter(|n| !n.is_empty()).map(String::from))
        .collect();
    names.sort();
    if names.is_empty() {
        return (
            "empty".into(),
            "Ollama läuft, aber es ist noch kein Modell installiert. Lade in Ollama ein Modell \
herunter („ollama pull <Modellname>“) und teste dann erneut."
                .into(),
            Vec::new(),
        );
    }
    let msg = format!("Ollama läuft. Installierte Modelle: {}.", names.len());
    ("ok".into(), msg, names)
}

/// Pegelanzeige für den Mikrofontest starten. Fehlertext bei Problemen.
#[tauri::command]
pub async fn mic_start(device: Option<String>, app: AppHandle) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut ws = wizard_state(&app);
        if let Some(mut m) = ws.monitor.take() {
            m.discard();
        }
        let mut rec = AudioRecorder::new(device);
        rec.start();
        if let Some(e) = rec.error.clone() {
            return Some(e);
        }
        ws.monitor = Some(rec);
        None
    })
    .await
    .unwrap_or_else(|e| Some(e.to_string()))
}

/// Aktueller Pegel 0..1 (gesammelte Daten werden verworfen).
#[tauri::command]
pub fn mic_level(app: AppHandle) -> f32 {
    let ws = wizard_state(&app);
    match &ws.monitor {
        Some(m) => {
            m.clear_buffer();
            m.audio_level()
        }
        None => 0.0,
    }
}

#[tauri::command]
pub fn mic_stop(app: AppHandle) {
    if let Some(mut m) = wizard_state(&app).monitor.take() {
        m.discard();
    }
}

/// Probediktat starten (mit den Einstellungen des Assistenten).
#[tauri::command]
pub async fn wizard_test_start(settings: AppSettings, app: AppHandle) {
    log::info!("Assistent: Probediktat starten");
    let handle = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let mut ws = wizard_state(&handle);
        if ws.runner.as_ref().is_some_and(|r| r.is_recording()) {
            return;
        }
        if let Some(mut old) = ws.runner.take() {
            old.cancel();
        }
        let emit = handle.clone();
        let on_phase: crate::workflow::PhaseCallback = Arc::new(move |phase: Phase, message: &str| {
            let _ = emit.emit("wizard-phase", json!({ "phase": phase.as_str(), "message": message }));
        });
        let mut runner = WorkflowRunner::new(WorkflowKind::Transcription, settings, Some(on_phase), None);
        runner.start_recording();
        ws.runner = Some(runner);
    })
    .await;
}

/// Probediktat beenden.
#[tauri::command]
pub async fn wizard_test_stop(app: AppHandle) {
    log::info!("Assistent: Probediktat stoppen");
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Some(r) = wizard_state(&app).runner.as_mut() {
            if r.is_recording() {
                r.stop_recording();
            }
        }
    })
    .await;
}

#[tauri::command]
pub fn wizard_test_recording(app: AppHandle) -> bool {
    wizard_state(&app).runner.as_ref().is_some_and(|r| r.is_recording())
}

/// Probediktat-Seite verlassen: Aufnahme abbrechen, Kürzel abmelden.
#[tauri::command]
pub fn wizard_test_leave(app: AppHandle) {
    let mut ws = wizard_state(&app);
    if let Some(mut r) = ws.runner.take() {
        r.cancel();
    }
    if let Some(mut h) = ws.hotkeys.take() {
        h.stop();
    }
}

/// Kürzel für das Probediktat einrichten; meldet „start“/„stop“ an die
/// Oberfläche. Fehlertext, wenn Tastenkürzel nicht verfügbar sind.
#[tauri::command]
pub fn wizard_hotkey(combo: String, mode: String, app: AppHandle) -> Option<String> {
    let mut ws = wizard_state(&app);
    if let Some(mut h) = ws.hotkeys.take() {
        h.stop();
    }
    let mut svc = HotkeyService::new();
    let emit = app.clone();
    log::info!("Assistent: Probediktat-Kürzel {combo:?} ({mode})");
    svc.register(&combo, Mode::from_setting(&mode), move |ev| {
        let action = match ev {
            HotkeyEvent::Pressed | HotkeyEvent::Toggled(true) => "start",
            HotkeyEvent::Released | HotkeyEvent::Toggled(false) => "stop",
        };
        log::info!("Assistent: Kürzel {action}");
        let _ = emit.emit("wizard-hotkey", action);
    });
    match svc.start() {
        Ok(()) => {
            ws.hotkeys = Some(svc);
            None
        }
        Err(e) => Some(e),
    }
}

/// „Fertig“: Autostart setzen, Einstellungen speichern, Tastenkürzel neu
/// einrichten, Hinweis zeigen und Fenster schließen.
#[tauri::command]
pub fn wizard_finish(mut settings: AppSettings, app: AppHandle) -> Result<(), String> {
    settings.has_seen_onboarding = true;
    if let Err(e) = autostart::set_enabled(settings.launch_at_login) {
        app.dialog()
            .message(format!("Der Autostart-Eintrag konnte nicht geändert werden:\n{e}"))
            .title("BlitzVox")
            .kind(MessageDialogKind::Warning)
            .show(|_| {});
    }
    save_settings(&settings).map_err(|e| e.to_string())?;
    *lock(&app.state::<AppState>().settings) = settings;
    wizard_state(&app).finished = true;
    notify(&app, "BlitzVox ist eingerichtet", "Klicke auf das Tray-Icon oder nutze die Tastenkürzel.");
    if let Some(w) = app.get_webview_window("wizard") {
        let _ = w.close(); // Tastenkürzel richtet on_closed ein
    }
    Ok(())
}

/// Link im Standardbrowser öffnen (nur http/https).
#[tauri::command]
pub fn open_url(url: String, app: AppHandle) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Ungültiger Link".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    /// Live-Test: zeigt die Vulkan-Grafikkarten dieses Rechners (darf nicht abstürzen).
    /// Aufruf: cargo test --release --lib vulkan_geraete -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vulkan_geraete() {
        println!("Vulkan-Geräte: {:?}", super::vulkan_gpu_names());
    }
}
