mod app;
mod autostart;
mod backup;
mod corrections;
mod hotkeys;
mod http;
#[cfg(target_os = "linux")]
mod linux_input;
mod llm;
mod local_stt;
mod paste;
mod recorder;
mod secrets;
mod settings;
mod settings_ui;
mod stt;
mod wizard;
mod workflow;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    app::run();
}
