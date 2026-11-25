//! Lokale Spracherkennung mit whisper.cpp (über whisper-rs, GPU per Vulkan).
//! Ersetzt `_transcribe_local` aus stt.py (dort faster-whisper).
//!
//! Modelle (GGML) werden beim ersten Gebrauch von Hugging Face geladen und in
//! `%LOCALAPPDATA%\blitzvox\models` (Windows) bzw.
//! `~/.local/share/blitzvox/models` (Linux) abgelegt – Entscheidung vom
//! 24.11.2026. Modellgrößen wie bisher: tiny, base, small, medium.
//!
//! Wie in 1.0.8: erst die Grafikkarte versuchen, bei Problemen automatisch
//! auf den Prozessor ausweichen. Erkennung mit Beam-Suche (Breite 5) und
//! Kontext aus vorherigen Abschnitten, wie die Voreinstellung von
//! faster-whisper.

// Wird erst mit den Workflows vollständig benutzt.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use tokio::io::AsyncWriteExt;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::recorder::{SAMPLE_RATE, resample_linear};

const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Ordner für die Modelle (wird bei Bedarf angelegt).
pub fn models_dir() -> std::io::Result<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("AppData").join("Local")))
            .unwrap_or_default()
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .unwrap_or_default()
    };
    let dir = base.join("blitzvox").join("models");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Dateiname des Modells, z. B. `ggml-base.bin`. Nur einfache Namen sind
/// erlaubt, damit aus der Einstellung kein fremder Pfad entstehen kann.
pub fn model_file_name(size: &str) -> Result<String, String> {
    let ok = !size.is_empty()
        && size.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
        && !size.contains("..");
    if ok { Ok(format!("ggml-{size}.bin")) } else { Err(format!("Ungültige Modellgröße: {size}")) }
}

pub fn model_path(size: &str) -> Result<PathBuf, String> {
    Ok(models_dir().map_err(|e| e.to_string())?.join(model_file_name(size)?))
}

pub fn is_model_downloaded(size: &str) -> bool {
    model_path(size).map(|p| p.is_file()).unwrap_or(false)
}

/// Lädt das Modell herunter, falls es fehlt. `progress(geladen, gesamt)`.
pub async fn ensure_model(size: &str, progress: impl Fn(u64, Option<u64>)) -> Result<PathBuf, String> {
    let path = model_path(size)?;
    if path.is_file() {
        return Ok(path);
    }
    let url = format!("{MODEL_BASE_URL}/{}", model_file_name(size)?);
    download(&url, &path, progress).await?;
    Ok(path)
}

async fn download(url: &str, target: &Path, progress: impl Fn(u64, Option<u64>)) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Modell-Download fehlgeschlagen: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Modell-Download fehlgeschlagen: HTTP {}", resp.status().as_u16()));
    }
    let total = resp.content_length();
    // Erst in eine Teildatei schreiben, damit nie ein halbes Modell liegen bleibt
    let part = target.with_extension("bin.part");
    let result = async {
        let mut file = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
        let mut done = 0u64;
        progress(0, total);
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Modell-Download abgebrochen: {e}"))? {
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        file.flush().await.map_err(|e| e.to_string())?;
        if let Some(t) = total {
            if done != t {
                return Err(format!("Modell-Download unvollständig ({done} von {t} Bytes)."));
            }
        }
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => tokio::fs::rename(&part, target).await.map_err(|e| e.to_string()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

// ── Erkennung ────────────────────────────────────────────────────────────────

/// Zuletzt geladenes Modell (Pfad, GPU ja/nein), damit es nicht bei jeder
/// Aufnahme neu geladen werden muss.
static CONTEXT: Mutex<Option<(PathBuf, bool, WhisperContext)>> = Mutex::new(None);
static LOGGING: Once = Once::new();

/// Erkennt die WAV-Datei mit dem Modell `size` (muss heruntergeladen sein).
/// `language`: leer = automatisch erkennen. `prompt`: Hinweistext.
pub fn transcribe_file(audio_path: &Path, size: &str, language: &str, prompt: &str) -> Result<String, String> {
    LOGGING.call_once(whisper_rs::install_logging_hooks); // Ausgaben von whisper.cpp nicht auf die Konsole
    let model = model_path(size)?;
    if !model.is_file() {
        return Err(format!("Das Modell „{size}“ ist noch nicht heruntergeladen."));
    }
    let audio = load_audio(audio_path)?;
    // Grafikkarte zuerst, bei einem Fehler auf den Prozessor ausweichen
    match run(&model, true, &audio, language, prompt) {
        Ok(t) => Ok(t),
        Err(gpu_err) => {
            log::warn!("Lokale Erkennung auf der Grafikkarte fehlgeschlagen: {gpu_err}");
            run(&model, false, &audio, language, prompt)
        }
    }
}

fn run(model: &Path, use_gpu: bool, audio: &[f32], language: &str, prompt: &str) -> Result<String, String> {
    let mut guard = CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
    let reuse = matches!(&*guard, Some((p, g, _)) if p == model && *g == use_gpu);
    if !reuse {
        *guard = None; // altes Modell freigeben, bevor das neue geladen wird
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu);
        let ctx = WhisperContext::new_with_params(model, params).map_err(|e| e.to_string())?;
        *guard = Some((model.to_path_buf(), use_gpu, ctx));
    }
    let (_, _, ctx) = guard.as_ref().expect("Modell geladen");
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;

    let mut fp = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
    fp.set_language(if language.is_empty() { None } else { Some(language) });
    if !prompt.is_empty() {
        fp.set_initial_prompt(prompt);
    }
    fp.set_no_context(false);
    fp.set_print_special(false);
    fp.set_print_progress(false);
    fp.set_print_realtime(false);
    fp.set_print_timestamps(false);
    state.full(fp, audio).map_err(|e| e.to_string())?;

    let mut parts = Vec::new();
    for seg in state.as_iter() {
        parts.push(seg.to_str_lossy().map_err(|e| e.to_string())?.into_owned());
    }
    // Wie Python: " ".join(s.text for s in segments).strip()
    Ok(parts.join(" ").trim().to_string())
}

/// Liest eine WAV-Datei (PCM 8/16/24/32 Bit oder Float32) als mono 16 kHz,
/// wie `_load_audio()` in stt.py (Kanäle mitteln, sonst linear umrechnen).
pub fn load_audio(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Keine WAV-Datei.".into());
    }
    let (mut format, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + len).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                format = u16::from_le_bytes([body[0], body[1]]);
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes([body[14], body[15]]);
                if format == 0xFFFE && body.len() >= 26 {
                    format = u16::from_le_bytes([body[24], body[25]]); // WAVE_FORMAT_EXTENSIBLE
                }
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let data = data.ok_or("WAV-Datei ohne Audiodaten.")?;
    if channels == 0 || rate == 0 {
        return Err("WAV-Datei ohne gültiges Format.".into());
    }
    let samples: Vec<f32> = match (format, bits) {
        (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        (1, 24) => data
            .chunks_exact(3)
            .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes(c.try_into().unwrap()) as f32 / 2_147_483_648.0)
            .collect(),
        (3, 32) => data.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
        _ => return Err(format!("Nicht unterstütztes WAV-Format ({format}, {bits} Bit).")),
    };
    let ch = channels as usize;
    let mono: Vec<f32> = if ch > 1 {
        samples.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    } else {
        samples
    };
    Ok(if rate != SAMPLE_RATE { resample_linear(&mono, rate, SAMPLE_RATE) } else { mono })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modellnamen() {
        assert_eq!(model_file_name("base").unwrap(), "ggml-base.bin");
        assert!(model_file_name("../x").is_err());
        assert!(model_file_name("C:\\x").is_err());
        assert!(model_file_name("").is_err());
    }

    #[test]
    fn wav_lesen_wie_geschrieben() {
        let p = std::env::temp_dir().join(format!("blitzvox-load-{}.wav", std::process::id()));
        crate::recorder::write_wav(&p, &[0.0, 0.5, -0.5], 16000).unwrap();
        let a = load_audio(&p).unwrap();
        assert_eq!(a.len(), 3);
        assert!((a[1] - 0.5).abs() < 1e-3 && (a[2] + 0.5).abs() < 1e-3);
        let _ = std::fs::remove_file(p);
    }

    /// Lädt das Modell „base“ (148 MB) herunter, falls nötig, und erkennt die
    /// Testdatei auf der Grafikkarte.
    #[tokio::test]
    #[ignore = "lädt ggf. 148 MB herunter und nutzt die GPU; manuell mit --ignored starten"]
    async fn lokal_live() {
        let path = ensure_model("base", |_, _| {}).await.unwrap();
        println!("Modell: {} ({} Bytes)", path.display(), std::fs::metadata(&path).unwrap().len());
        let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sprache_de.wav");
        let start = std::time::Instant::now();
        let t = tokio::task::spawn_blocking(move || transcribe_file(&wav, "base", "de", "")).await.unwrap().unwrap();
        println!("Lokal erkannt in {:.2} s: {t}", start.elapsed().as_secs_f64());
        let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sprache_de.wav");
        let start = std::time::Instant::now();
        let t2 = tokio::task::spawn_blocking(move || transcribe_file(&wav, "base", "de", "")).await.unwrap().unwrap();
        println!("Zweiter Durchlauf (Modell schon geladen) in {:.2} s: {t2}", start.elapsed().as_secs_f64());
        assert!(t.to_lowercase().contains("test"));

        // Vergleich: nur Prozessor
        let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sprache_de.wav");
        let audio = load_audio(&wav).unwrap();
        let model = model_path("base").unwrap();
        let _ = run(&model, false, &audio, "de", ""); // Laden
        let start = std::time::Instant::now();
        let t3 = run(&model, false, &audio, "de", "").unwrap();
        println!("Nur Prozessor (Modell geladen) in {:.2} s: {t3}", start.elapsed().as_secs_f64());
    }
}
