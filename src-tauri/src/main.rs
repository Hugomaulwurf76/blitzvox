// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
  // Linux: WebKitGTK zeichnet mit dem DMA-BUF-Renderer auf manchen
  // Grafiktreibern nur weiße, nicht bedienbare Fenster (beobachtet unter
  // Linux Mint 22.3 in VirtualBox, 25.11.2026). Ohne ihn geht es. Wer die
  // Variable selbst setzt, behält seinen Wert.
  #[cfg(target_os = "linux")]
  if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
    // SAFETY: Noch kein weiterer Thread gestartet – erster Schritt des Programms.
    unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
  }
  blitzvox_lib::run();
}
