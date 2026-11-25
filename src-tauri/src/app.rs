//! Programmrahmen: Tray-Symbol und -Menü, Aufnahmefenster, Tastenkürzel,
//! Fehlerprotokoll – Nachbau von `ui/tray.py`, `ui/workflow_window.py`
//! (Steuerung) und `main.py`.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;

use crate::corrections;
use crate::hotkeys::{self, HotkeyAction, HotkeyService, HotkeyWorkflow};
use crate::paste;
use crate::settings::{self, AppSettings};
use crate::workflow::{Phase, WorkflowKind, WorkflowRunner};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Workflow-Metadaten (wie WORKFLOWS in tray.py) ────────────────────────────

pub struct WorkflowMeta {
    pub kind: WorkflowKind,
    pub id: &'static str,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub icon: &'static str,
}

pub const WORKFLOWS: [WorkflowMeta; 4] = [
    WorkflowMeta {
        kind: WorkflowKind::Transcription,
        id: "transcription",
        label: "BlitzVox",
        subtitle: "Sprache rein. Text raus.",
        icon: "🎙",
    },
    WorkflowMeta {
        kind: WorkflowKind::TextImprove,
        id: "text_improve",
        label: "BlitzVox+",
        subtitle: "Geschrieben sprechen.",
        icon: "✨",
    },
    WorkflowMeta {
        kind: WorkflowKind::DampfAblassen,
        id: "dampf_ablassen",
        label: "BlitzVox $%&!",
        subtitle: "Frust rein. Entspannt raus.",
        icon: "🔥",
    },
    WorkflowMeta {
        kind: WorkflowKind::EmojiText,
        id: "emoji_text",
        label: "BlitzVox :)",
        subtitle: "Text rein. Emojis dazu.",
        icon: "😊",
    },
];

fn meta(kind: WorkflowKind) -> &'static WorkflowMeta {
    WORKFLOWS.iter().find(|m| m.kind == kind).unwrap_or(&WORKFLOWS[0])
}

fn meta_by_id(id: &str) -> Option<&'static WorkflowMeta> {
    WORKFLOWS.iter().find(|m| m.id == id)
}

fn window_label(kind: WorkflowKind) -> String {
    format!("workflow-{}", meta(kind).id)
}

fn hotkey_kind(w: HotkeyWorkflow) -> WorkflowKind {
    match w {
        HotkeyWorkflow::Transcription => WorkflowKind::Transcription,
        HotkeyWorkflow::TextImprovement => WorkflowKind::TextImprove,
        HotkeyWorkflow::DampfAblassen => WorkflowKind::DampfAblassen,
        HotkeyWorkflow::EmojiText => WorkflowKind::EmojiText,
    }
}

// ── Zustand ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AppState {
    pub(crate) settings: Mutex<AppSettings>,
    runners: Mutex<HashMap<WorkflowKind, WorkflowRunner>>,
    hotkeys: Mutex<Option<HotkeyService>>,
    tray: Mutex<Option<TrayIcon>>,
    tray_icons: OnceLock<(Image<'static>, Image<'static>)>,
    /// Nummer des aktuellen Fensters je Workflow
    window_ids: Mutex<HashMap<WorkflowKind, u64>>,
    /// Text für das Korrekturfenster
    correction_text: Mutex<String>,
    /// Laufende Dinge des Einrichtungsassistenten
    pub(crate) wizard: Mutex<crate::wizard::WizardState>,
}

static NEXT_WINDOW_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static APP: OnceLock<AppHandle> = OnceLock::new();

// ── Fehlerprotokoll (wie _show_unhandled in main.py) ─────────────────────────

fn write_error_log(text: &str) {
    if let Ok(dir) = settings::config_dir() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("error.log")) {
            let _ = writeln!(f, "{text}\n");
        }
    }
}

/// Unbehandelte Fehler in `error.log` schreiben und als Fenster zeigen,
/// statt dass das Programm ohne Meldung endet.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("{info}\n\n{bt}");
        write_error_log(&text);
        if let Some(app) = APP.get() {
            app.dialog().message(text).title("BlitzVox – Fehler").kind(MessageDialogKind::Error).show(|_| {});
        }
    }));
}

// ── Fokus ────────────────────────────────────────────────────────────────────

/// Fenster nach vorn holen – aber nur, wenn BlitzVox schon im Vordergrund ist
/// (z. B. nach einem Klick ins Tray-Menü).
///
/// Hintergrund: Ist ein anderes Programm vorn (Start per Tastenkürzel), greift
/// Tauri zu einem Trick und schickt per `SendInput` einen künstlichen
/// Alt-Tastendruck. Danach ruft Windows unseren Tastatur-Hook nicht mehr auf,
/// und die Tastenkürzel reagieren nicht mehr (Aufnahme ließ sich nicht per
/// Loslassen bzw. zweitem Druck stoppen; nachgestellt am 24.11.2026).
/// Das Fenster erscheint trotzdem (immer im Vordergrund), der Fokus bleibt im
/// Zielprogramm – dorthin wird ohnehin eingefügt.
pub(crate) fn focus_window(w: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        let mut pid = 0u32;
        // SAFETY: reine Abfragen, Ausgabezeiger zeigt auf eine lokale Variable.
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
        if pid != std::process::id() {
            return;
        }
    }
    let _ = w.set_focus();
}

// ── Tray ─────────────────────────────────────────────────────────────────────

/// Tray-Symbol aus `blitzvox.png`; bei Aufnahme mit rotem Punkt unten rechts
/// (Entscheidung vom 24.11.2026; 1.0.8 färbte das Mikrofon rot).
fn make_tray_icons() -> (Image<'static>, Image<'static>) {
    let base = Image::from_bytes(include_bytes!("../icons/64x64.png")).expect("Tray-Symbol").to_owned();
    let (w, h) = (base.width(), base.height());
    let mut rgba = base.rgba().to_vec();
    // Roter Punkt mit weißem Rand, weich gezeichnet
    let (cx, cy) = (w as f32 * 0.76, h as f32 * 0.76);
    let r = w as f32 * 0.22;
    let ring = w as f32 * 0.05;
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let (color, cover) = if d <= r - ring {
                ([0xe7u8, 0x4c, 0x3c], (r - ring - d).clamp(0.0, 1.0))
            } else {
                ([0xffu8, 0xff, 0xff], (r - d).clamp(0.0, 1.0))
            };
            // Innenkante: Rot über Weiß weich überblenden
            let (color, cover) = if d > r - ring - 1.0 && d <= r - ring {
                let t = r - ring - d; // 0..1
                let mix = |a: u8, b: u8| (a as f32 * t + b as f32 * (1.0 - t)) as u8;
                ([mix(0xe7, 0xff), mix(0x4c, 0xff), mix(0x3c, 0xff)], 1.0f32.min(r - d))
            } else {
                (color, cover)
            };
            if cover <= 0.0 {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                rgba[i + c] = (color[c] as f32 * cover + rgba[i + c] as f32 * (1.0 - cover)) as u8;
            }
            rgba[i + 3] = (255.0 * cover + rgba[i + 3] as f32 * (1.0 - cover)) as u8;
        }
    }
    (base, Image::new_owned(rgba, w, h))
}

fn set_tray_recording(app: &AppHandle, recording: bool) {
    let state = app.state::<AppState>();
    let Some((normal, rec)) = state.tray_icons.get() else { return };
    if let Some(tray) = lock(&state.tray).as_ref() {
        let _ = tray.set_icon(Some(if recording { rec.clone() } else { normal.clone() }));
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    for wf in &WORKFLOWS {
        let text = format!("{}  {}  —  {}", wf.icon, wf.label, wf.subtitle);
        menu.append(&MenuItem::with_id(app, format!("wf:{}", wf.id), text, true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "correction", "✏  Letzten Text korrigieren …", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "⚙  Einstellungen", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?)?;

    let state = app.state::<AppState>();
    let icons = state.tray_icons.get_or_init(make_tray_icons);
    let tray = TrayIconBuilder::with_id("blitzvox")
        .icon(icons.0.clone())
        .tooltip(format!("BlitzVox v{VERSION}"))
        .menu(&menu)
        // Linksklick öffnet das Menü nur unter Windows (wie 1.0.8)
        .show_menu_on_left_click(cfg!(windows))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "correction" => open_correction(app),
            "settings" => open_settings(app),
            "quit" => app.exit(0),
            id => {
                if let Some(wf) = id.strip_prefix("wf:").and_then(meta_by_id) {
                    open_workflow(app, wf.kind, false);
                }
            }
        })
        .build(app)?;
    *lock(&state.tray) = Some(tray);
    Ok(())
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

// ── Korrekturfenster (wie TrayApp._open_correction) ──────────────────────────

fn open_correction(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("correction") {
        if w.is_visible().unwrap_or(false) {
            focus_window(&w);
            return;
        }
        let _ = w.destroy();
    }
    let text = corrections::get_last();
    if text.is_empty() {
        notify(app, "BlitzVox", "Es wurde noch kein Text diktiert.");
        return;
    }
    *lock(&app.state::<AppState>().correction_text) = text;
    let win = WebviewWindowBuilder::new(app, "correction", WebviewUrl::App("correction.html".into()))
        .title("BlitzVox – Text korrigieren")
        .inner_size(480.0, 360.0)
        .min_inner_size(480.0, 200.0)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .center()
        .build();
    match win {
        Ok(w) => {
            focus_window(&w);
        }
        Err(e) => log::error!("Korrekturfenster konnte nicht geöffnet werden: {e}"),
    }
}

/// Text, mit dem das Korrekturfenster geöffnet wurde.
#[tauri::command]
fn corr_text(state: State<'_, AppState>) -> String {
    lock(&state.correction_text).clone()
}

/// Lern-Vorschläge (falsch, richtig) aus der Bearbeitung.
#[tauri::command]
fn corr_suggest(original: String, edited: String) -> Vec<(String, String)> {
    corrections::suggest_from_edit(&original, &edited)
}

/// „Übernehmen & kopieren“: angehakte Paare lernen, Text kopieren und als
/// letzten Text merken.
#[tauri::command]
fn corr_apply(learned: Vec<(String, String)>, text: String) -> Result<(), String> {
    for (wrong, right) in &learned {
        corrections::add_replacement(wrong, right).map_err(|e| e.to_string())?;
    }
    paste::copy_to_clipboard(&text)?;
    corrections::set_last(&text);
    Ok(())
}

/// Schließt das aufrufende Fenster.
#[tauri::command]
fn close_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

// ── Einstellungen und Einrichtungsassistent ──────────────────────────────────

fn open_settings(app: &AppHandle) {
    crate::settings_ui::open_settings(app);
}

/// Einrichtungsassistent. `automatic`: beim Programmstart.
pub(crate) fn open_setup_wizard(app: &AppHandle, automatic: bool) {
    crate::wizard::open(app, automatic);
}

// ── Tastenkürzel (wie TrayApp._setup_hotkeys / _hotkey_start / _hotkey_stop) ─

/// Tastenkürzel abschalten (z. B. während des Einrichtungsassistenten).
pub(crate) fn stop_hotkeys(app: &AppHandle) {
    if let Some(mut old) = lock(&app.state::<AppState>().hotkeys).take() {
        old.stop();
    }
}

pub(crate) fn setup_hotkeys(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(mut old) = lock(&state.hotkeys).take() {
        old.stop();
    }
    let hk = lock(&state.settings).hotkeys.clone();
    let handle = app.clone();
    match hotkeys::start_from_settings(&hk, move |action| {
        let app = handle.clone();
        log::info!("Tastenkürzel: {action:?}");
        // Fensteraktionen nicht im Tastatur-Thread ausführen
        std::thread::spawn(move || match action {
            HotkeyAction::Start(w) => hotkey_start(&app, hotkey_kind(w)),
            HotkeyAction::Stop(w) => hotkey_stop(&app, hotkey_kind(w)),
            HotkeyAction::Correction => open_correction(&app),
        });
    }) {
        Ok(svc) => {
            *lock(&state.hotkeys) = svc;
            // Linux/Wayland ohne Tastaturzugriff und ohne Portal: einmal darauf hinweisen
            static HINT_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let Some(reason) = hotkeys::limited_reason() {
                if !HINT_SHOWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    notify(
                        app,
                        "BlitzVox – Tastenkürzel eingeschränkt",
                        &format!("Die Tastenkürzel wirken nur in X11-Programmen. {reason}"),
                    );
                }
            }
        }
        Err(e) => log::warn!("Tastenkürzel nicht verfügbar: {e}"),
    }
}

/// Kürzel gedrückt – Fenster öffnen (falls nötig) und Aufnahme starten.
fn hotkey_start(app: &AppHandle, kind: WorkflowKind) {
    let visible = app
        .get_webview_window(&window_label(kind))
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        start_recording_external(app, kind);
    } else {
        open_workflow(app, kind, true);
    }
}

/// Kürzel losgelassen bzw. Umschalten aus – Aufnahme stoppen.
fn hotkey_stop(app: &AppHandle, kind: WorkflowKind) {
    stop_recording_external(app, kind);
}

// ── Aufnahmefenster ──────────────────────────────────────────────────────────

fn open_workflow(app: &AppHandle, kind: WorkflowKind, auto_record: bool) {
    let state = app.state::<AppState>();
    // Einstellungen vor dem Öffnen frisch laden (wie 1.0.8)
    let fresh = settings::load_settings();
    *lock(&state.settings) = fresh.clone();

    let label = window_label(kind);
    if let Some(old) = app.get_webview_window(&label) {
        let _ = old.destroy();
    }
    if let Some(mut old) = lock(&state.runners).remove(&kind) {
        old.cancel();
    }

    let emit_app = app.clone();
    let on_phase: crate::workflow::PhaseCallback = Arc::new(move |phase, message| {
        on_phase(&emit_app, kind, phase, message);
    });
    let runner = WorkflowRunner::new(kind, fresh, Some(on_phase), None);
    lock(&state.runners).insert(kind, runner);

    let window_id = NEXT_WINDOW_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    lock(&state.window_ids).insert(kind, window_id);

    paste::save_focus();
    let m = meta(kind);
    let url = WebviewUrl::App(format!("workflow.html?kind={}", m.id).into());
    let win = WebviewWindowBuilder::new(app, &label, url)
        .title(m.label)
        .inner_size(400.0, 260.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .center()
        .build();
    match win {
        Ok(w) => {
            let handle = app.clone();
            w.on_window_event(move |ev| {
                if let tauri::WindowEvent::Destroyed = ev {
                    let state = handle.state::<AppState>();
                    // Ist schon ein neues Fenster für diesen Workflow offen, gehört
                    // der Runner diesem – dann nichts abbrechen.
                    if lock(&state.window_ids).get(&kind) != Some(&window_id) {
                        return;
                    }
                    // Schließen bricht Aufnahme/Verarbeitung ab (closeEvent in 1.0.8)
                    if let Some(r) = lock(&state.runners).get_mut(&kind) {
                        r.cancel();
                    }
                    set_tray_recording(&handle, false);
                }
            });
            focus_window(&w);
        }
        Err(e) => log::error!("Fenster konnte nicht geöffnet werden: {e}"),
    }

    if auto_record {
        // kurz warten, bis das Fenster da ist, dann Aufnahme starten
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            start_recording_external(&app, kind);
        });
    }
}

fn start_recording_external(app: &AppHandle, kind: WorkflowKind) {
    let state = app.state::<AppState>();
    let mut runners = lock(&state.runners);
    if let Some(r) = runners.get_mut(&kind) {
        log::info!("Aufnahme starten ({kind:?}): Phase {:?}", r.phase());
        if matches!(r.phase(), Phase::Idle | Phase::Done | Phase::Error) {
            r.reset();
            r.start_recording();
        }
    }
}

fn stop_recording_external(app: &AppHandle, kind: WorkflowKind) {
    let state = app.state::<AppState>();
    let mut runners = lock(&state.runners);
    match runners.get_mut(&kind) {
        Some(r) => {
            log::info!("Aufnahme stoppen ({kind:?}): Phase {:?}", r.phase());
            if r.phase() == Phase::Recording {
                r.stop_recording();
            }
        }
        None => log::info!("Aufnahme stoppen ({kind:?}): kein Runner"),
    }
}

/// Phasenwechsel an das Fenster melden, Tray-Symbol anpassen und bei
/// „fertig“ Zwischenablage und Einfügen erledigen (wie `_set_done`).
fn on_phase(app: &AppHandle, kind: WorkflowKind, phase: Phase, message: &str) {
    let auto_paste = lock(&app.state::<AppState>().settings).auto_paste;
    let _ = app.emit(
        "wf-phase",
        json!({ "kind": meta(kind).id, "phase": phase.as_str(), "message": message, "autoPaste": auto_paste }),
    );
    set_tray_recording(app, phase == Phase::Recording);

    if phase == Phase::Done {
        let text = message.to_string();
        if let Err(e) = paste::copy_to_clipboard(&text) {
            log::warn!("Zwischenablage: {e}");
        }
        if auto_paste {
            // Kurz warten, Fenster verstecken, damit der Fokus ans vorherige
            // Fenster zurückfällt, dann einfügen und schließen
            let app = app.clone();
            std::thread::spawn(move || {
                let win = app.get_webview_window(&window_label(kind));
                std::thread::sleep(Duration::from_millis(150));
                if let Some(w) = &win {
                    let _ = w.hide();
                }
                std::thread::sleep(Duration::from_millis(180));
                paste::paste_to_focused_window(&text);
                std::thread::sleep(Duration::from_millis(100));
                if let Some(w) = &win {
                    let _ = w.close();
                }
            });
        }
    }
}

// ── Befehle für die Oberfläche ───────────────────────────────────────────────

fn kind_of(id: &str) -> Result<WorkflowKind, String> {
    meta_by_id(id).map(|m| m.kind).ok_or_else(|| format!("Unbekannter Workflow: {id}"))
}

/// Daten zum Aufbau des Fensters und aktueller Zustand.
#[tauri::command]
fn wf_info(kind: String, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    log::info!("Fenster meldet Workflow: {kind}");
    let k = kind_of(&kind)?;
    let m = meta(k);
    let phase = lock(&state.runners).get(&k).map(|r| r.phase()).unwrap_or(Phase::Idle);
    Ok(json!({ "label": m.label, "subtitle": m.subtitle, "icon": m.icon, "phase": phase.as_str() }))
}

/// Knopf „Aufnahme starten/stoppen“ (wie `_toggle_recording`).
#[tauri::command]
async fn wf_toggle(kind: String, app: AppHandle) -> Result<(), String> {
    let k = kind_of(&kind)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut runners = lock(&state.runners);
        if let Some(r) = runners.get_mut(&k) {
            match r.phase() {
                Phase::Idle | Phase::Done | Phase::Error => {
                    r.reset();
                    r.start_recording();
                }
                Phase::Recording => r.stop_recording(),
                _ => {}
            }
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Pegel für die Wellenform (0..1), `null` wenn keine Aufnahme läuft.
#[tauri::command]
fn wf_level(kind: String, state: State<'_, AppState>) -> Result<Option<f32>, String> {
    let k = kind_of(&kind)?;
    let runners = lock(&state.runners);
    Ok(runners.get(&k).filter(|r| r.is_recording()).map(|r| r.audio_level()))
}

/// Knopf „Abbrechen“: abbrechen und Fenster schließen.
#[tauri::command]
fn wf_cancel(kind: String, app: AppHandle) -> Result<(), String> {
    let k = kind_of(&kind)?;
    if let Some(r) = lock(&app.state::<AppState>().runners).get_mut(&k) {
        r.cancel();
    }
    if let Some(w) = app.get_webview_window(&window_label(k)) {
        let _ = w.close();
    }
    Ok(())
}

/// Tastenereignis aus einem BlitzVox-Fenster (siehe `hotkeys::inject_key`).
#[tauri::command]
fn key_event(down: bool, key: String) {
    hotkeys::inject_key(down, &key);
}

/// Knopf „Kopieren“.
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    paste::copy_to_clipboard(&text)
}

/// Fensterhöhe an den Inhalt anpassen (wie `adjustSize`).
#[tauri::command]
fn fit_height(height: f64, window: tauri::WebviewWindow) -> Result<(), String> {
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let width = size.width as f64 / scale;
    window
        .set_size(tauri::LogicalSize::new(width, height.ceil()))
        .map_err(|e| e.to_string())
}

// ── Start ────────────────────────────────────────────────────────────────────

pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            app.dialog()
                .message("BlitzVox läuft bereits. Das Symbol findest du in der Taskleiste (Tray).")
                .title("BlitzVox")
                .show(|_| {});
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            wf_info,
            wf_toggle,
            wf_level,
            wf_cancel,
            key_event,
            copy_text,
            fit_height,
            corr_text,
            corr_suggest,
            corr_apply,
            close_window,
            crate::settings_ui::settings_meta,
            crate::settings_ui::settings_get,
            crate::settings_ui::keys_status,
            crate::settings_ui::key_get,
            crate::settings_ui::key_clear,
            crate::settings_ui::clipboard_read,
            crate::settings_ui::openrouter_models,
            crate::settings_ui::openrouter_stt_models,
            crate::settings_ui::input_devices,
            crate::settings_ui::settings_save,
            crate::settings_ui::corr_list,
            crate::settings_ui::corr_add,
            crate::settings_ui::corr_remove,
            crate::settings_ui::request_setup_wizard,
            crate::wizard::wizard_init,
            crate::wizard::key_check,
            crate::wizard::key_store,
            crate::wizard::key_masked,
            crate::wizard::ollama_check,
            crate::wizard::mic_start,
            crate::wizard::mic_level,
            crate::wizard::mic_stop,
            crate::wizard::wizard_test_start,
            crate::wizard::wizard_test_stop,
            crate::wizard::wizard_test_recording,
            crate::wizard::wizard_test_leave,
            crate::wizard::wizard_hotkey,
            crate::wizard::wizard_finish,
            crate::wizard::open_url
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    // Nur in die Datei (%LOCALAPPDATA%\<Kennung>\logs\BlitzVox.log):
                    // Ausgabe in ein Konsolenfenster kann Programmteile anhalten,
                    // solange dort Text markiert ist.
                    tauri_plugin_log::Builder::default()
                        .clear_targets()
                        .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                            file_name: None,
                        }))
                        .level(log::LevelFilter::Info)
                        // Testschalter: ausführliches Tastenprotokoll (nur Sondertasten)
                        .level_for(
                            "blitzvox_lib::hotkeys",
                            if std::env::var("BLITZVOX_TEST_DEBUG").is_ok() {
                                log::LevelFilter::Debug
                            } else {
                                log::LevelFilter::Info
                            },
                        )
                        .build(),
                )?;
            }
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());

            *lock(&app.state::<AppState>().settings) = settings::load_settings();
            if let Some(err) = settings::get_load_error() {
                handle
                    .dialog()
                    .message(format!(
                        "Die Einstellungsdatei konnte nicht gelesen werden:\n\n{err}\n\n\
BlitzVox startet mit den Standardwerten. settings.json wird jetzt mit \
den Standardwerten neu geschrieben. Die kaputte Datei liegt im oben \
genannten Sicherungspfad."
                    ))
                    .title("BlitzVox – Einstellungen nicht lesbar")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }

            build_tray(&handle)?;
            setup_hotkeys(&handle);
            // Einrichtungsassistent beim ersten Start bzw. wenn die
            // Spracherkennung nicht einsatzbereit ist
            let needs = crate::wizard::needs_setup(&lock(&app.state::<AppState>().settings));
            if needs {
                open_setup_wizard(&handle, true);
            }
            // Nur für Tests im Entwicklungsbuild: nach N Sekunden „Beenden“ auslösen
            #[cfg(debug_assertions)]
            if let Some(secs) = std::env::var("BLITZVOX_TEST_QUIT_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(secs));
                    log::info!("Test: Beenden wird ausgelöst");
                    h.exit(0);
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("BlitzVox konnte nicht gestartet werden")
        .run(|_app, event| {
            // Tray-Programm: weiterlaufen, auch wenn alle Fenster zu sind
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
