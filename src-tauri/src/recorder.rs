//! Audioaufnahme – Nachbau von `BlitzVox alt/blitzvox/services/recorder.py`
//! (dort mit sounddevice/PortAudio, hier mit cpal).
//!
//! Aufgenommen wird mit der natürlichen Abtastrate des Mikrofons (manche
//! USB-Mikrofone können nur 44,1/48 kHz). Beim Stoppen wird linear auf
//! 16 kHz umgerechnet (wie `np.interp`) und als WAV (16 Bit, mono) in eine
//! temporäre Datei geschrieben. Der Pegel 0..1 dient der Wellenform.
//!
//! Abweichungen von 1.0.8:
//! - Geräteliste: cpal nutzt unter Windows nur WASAPI, unter Linux ALSA.
//!   PortAudio listete jedes Mikrofon zusätzlich über MME, DirectSound und
//!   WDM-KS auf. Ein Gerät wird deshalb über seine stabile cpal-ID gewählt,
//!   nicht über die alte PortAudio-Nummer.
//! - Mehrkanal-Mikrofone werden zu mono gemittelt (PortAudio lieferte mono
//!   direkt; wie es dabei umrechnete, ist nicht dokumentiert).

// Wird erst mit Workflows und Oberfläche vollständig benutzt.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Whisper arbeitet am besten mit 16 kHz.
pub const SAMPLE_RATE: u32 = 16000;
/// Blockgröße für die Pegelberechnung (wie `blocksize=1024` in Python).
const LEVEL_BLOCK: usize = 1024;

/// Ein Aufnahmegerät für die Auswahl in Einstellungen und Assistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    /// Stabile ID (cpal `DeviceId` als Text)
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub default: bool,
}

/// Verfügbare Aufnahmegeräte. Bei Fehlern eine leere Liste.
pub fn list_input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok());
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?;
            let channels = d.default_input_config().map(|c| c.channels()).unwrap_or(0);
            Some(InputDevice {
                name: d.to_string(),
                default: Some(&id) == default_id.as_ref(),
                id: id.to_string(),
                channels,
            })
        })
        .collect()
}

fn find_device(id: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(id) = id {
        if let Ok(id) = id.parse::<cpal::DeviceId>() {
            if let Some(d) = host.device_by_id(&id) {
                return Ok(d);
            }
        }
        // Gerät nicht (mehr) vorhanden -> Standardgerät, wie bei None
    }
    host.default_input_device().ok_or_else(|| "Kein Aufnahmegerät gefunden.".to_string())
}

/// Gemeinsamer Zustand zwischen Aufnahme-Callback und Recorder.
#[derive(Default)]
struct Shared {
    frames: Mutex<Vec<f32>>,
    /// Pegel als f32-Bits
    level: AtomicU32,
    /// Teilblock für die Pegelberechnung: Summe der Quadrate und Anzahl
    block: Mutex<(f64, usize)>,
}

impl Shared {
    fn push(&self, mono: &[f32]) {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(mono);
        let mut block = self.block.lock().unwrap_or_else(|e| e.into_inner());
        for &s in mono {
            block.0 += (s as f64) * (s as f64);
            block.1 += 1;
            if block.1 == LEVEL_BLOCK {
                let rms = (block.0 / LEVEL_BLOCK as f64).sqrt() as f32;
                self.set_level((rms * 10.0).min(1.0)); // grobe Normierung wie in Python
                *block = (0.0, 0);
            }
        }
    }

    fn set_level(&self, v: f32) {
        self.level.store(v.to_bits(), Ordering::Relaxed);
    }

    fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

pub struct AudioRecorder {
    device: Option<String>,
    shared: Arc<Shared>,
    /// Beendet den Aufnahme-Thread (Stream wird dort gehalten)
    stop_tx: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
    device_rate: u32,
    start_time: Option<Instant>,
    duration: f64,
    tmp_path: Option<PathBuf>,
    pub error: Option<String>,
}

impl AudioRecorder {
    /// `device`: ID aus `list_input_devices()`, `None` = Standardgerät.
    pub fn new(device: Option<String>) -> Self {
        Self {
            device,
            shared: Arc::new(Shared::default()),
            stop_tx: None,
            worker: None,
            device_rate: SAMPLE_RATE,
            start_time: None,
            duration: 0.0,
            tmp_path: None,
            error: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.stop_tx.is_some()
    }

    /// RMS-Pegel 0..1 für die Wellenform.
    pub fn audio_level(&self) -> f32 {
        self.shared.level()
    }

    /// Dauer der letzten Aufnahme in Sekunden.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn start(&mut self) {
        if self.is_recording() {
            return;
        }
        self.error = None;
        self.shared = Arc::new(Shared::default());
        self.start_time = Some(Instant::now());

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let device = self.device.clone();
        let shared = self.shared.clone();
        // Der Stream bleibt in einem eigenen Thread, weil er nicht auf allen
        // Systemen zwischen Threads verschoben werden darf.
        let worker = thread::Builder::new().name("blitzvox-recorder".into()).spawn(move || {
            let stream = match open_stream(device.as_deref(), shared) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    stream
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = stop_rx.recv(); // bis stop()/discard() oder Recorder weg
            drop(stream);
        });
        let worker = match worker {
            Ok(w) => w,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(rate)) => {
                self.device_rate = rate;
                self.stop_tx = Some(stop_tx);
                self.worker = Some(worker);
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                let _ = worker.join();
            }
            Err(_) => {
                self.error = Some("Das Mikrofon reagiert nicht.".into());
                drop(stop_tx);
            }
        }
    }

    fn stop_stream(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }

    /// Stoppt die Aufnahme, schreibt die WAV-Datei und gibt ihren Pfad zurück
    /// (bei Fehler `None`, Grund in `error`).
    pub fn stop(&mut self) -> Option<PathBuf> {
        if !self.is_recording() {
            return None;
        }
        self.duration = self.start_time.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        self.stop_stream();
        self.shared.set_level(0.0);

        let audio = std::mem::take(&mut *self.shared.frames.lock().unwrap_or_else(|e| e.into_inner()));
        if audio.is_empty() {
            self.error = Some("Keine Audiodaten aufgenommen.".into());
            return None;
        }
        let audio = if self.device_rate != SAMPLE_RATE {
            resample_linear(&audio, self.device_rate, SAMPLE_RATE)
        } else {
            audio
        };

        let path = match temp_wav_path() {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e.to_string());
                return None;
            }
        };
        self.tmp_path = Some(path.clone());
        match write_wav(&path, &audio, SAMPLE_RATE) {
            Ok(()) => Some(path),
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// Für reine Pegelanzeigen (Mikrofon-Test im Assistenten): bisher
    /// gesammelte Audiodaten verwerfen, damit nichts aufläuft.
    pub fn clear_buffer(&self) {
        self.shared.frames.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Verwirft die Aufnahme und löscht eine bereits geschriebene Datei.
    pub fn discard(&mut self) {
        self.stop_stream();
        self.shared.frames.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.shared.set_level(0.0);
        if let Some(p) = self.tmp_path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        self.stop_stream();
    }
}

/// Zu kurze Aufnahmen werden verworfen.
pub fn is_too_short(duration: f64) -> bool {
    is_too_short_min(duration, 0.4)
}

pub fn is_too_short_min(duration: f64, min_seconds: f64) -> bool {
    duration < min_seconds
}

// ── Stream ───────────────────────────────────────────────────────────────────

fn open_stream(device: Option<&str>, shared: Arc<Shared>) -> Result<(cpal::Stream, u32), String> {
    let device = find_device(device)?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate();
    let channels = config.channels().max(1) as usize;
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, stream_config, channels, shared),
        SampleFormat::F64 => build::<f64>(&device, stream_config, channels, shared),
        SampleFormat::I8 => build::<i8>(&device, stream_config, channels, shared),
        SampleFormat::I16 => build::<i16>(&device, stream_config, channels, shared),
        SampleFormat::I32 => build::<i32>(&device, stream_config, channels, shared),
        SampleFormat::I64 => build::<i64>(&device, stream_config, channels, shared),
        SampleFormat::U8 => build::<u8>(&device, stream_config, channels, shared),
        SampleFormat::U16 => build::<u16>(&device, stream_config, channels, shared),
        SampleFormat::U32 => build::<u32>(&device, stream_config, channels, shared),
        SampleFormat::U64 => build::<u64>(&device, stream_config, channels, shared),
        other => return Err(format!("Nicht unterstütztes Audioformat: {other:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut mono: Vec<f32> = Vec::new();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                mono.clear();
                mono.extend(data.chunks(channels).map(|frame| {
                    frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / frame.len() as f32
                }));
                shared.push(&mono);
            },
            |err| log::warn!("Audiofehler: {err}"),
            Some(Duration::from_secs(5)),
        )
        .map_err(|e| e.to_string())
}

// ── Umrechnung und WAV ───────────────────────────────────────────────────────

/// Lineares Resampling exakt wie in recorder.py:
/// `np.interp(linspace(0,1,n_out,False), linspace(0,1,n_in,False), audio)`.
pub fn resample_linear(audio: &[f32], from: u32, to: u32) -> Vec<f32> {
    let n_in = audio.len();
    // Python: int(round(n_in * SAMPLE_RATE / device_rate)) – Runden auf gerade Zahl
    let n_out = (n_in as f64 * to as f64 / from as f64).round_ties_even() as usize;
    if n_out == 0 || n_in == 0 {
        return audio.to_vec();
    }
    let last = n_in - 1;
    (0..n_out)
        .map(|j| {
            let x = j as f64 / n_out as f64; // Position 0..1
            let pos = x * n_in as f64; // in Einheiten von x_old
            let i = pos.floor() as usize;
            if i >= last {
                return audio[last]; // np.interp hält den letzten Wert
            }
            let frac = pos - i as f64;
            (audio[i] as f64 + (audio[i + 1] as f64 - audio[i] as f64) * frac) as f32
        })
        .collect()
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Eindeutige temporäre `.wav`-Datei (wie `NamedTemporaryFile(suffix=".wav")`).
fn temp_wav_path() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for _ in 0..100 {
        let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("tmp{:x}{:x}{n}.wav", std::process::id(), nanos));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("Keine temporäre Datei möglich."))
}

/// Schreibt mono 16-Bit-PCM wie soundfile/libsndfile (Faktor 32767,
/// Runden auf gerade Zahl). Werte außerhalb von -1..1 werden begrenzt.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // Länge fmt-Block
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // Bytes pro Sekunde
    out.extend_from_slice(&2u16.to_le_bytes()); // Bytes pro Frame
    out.extend_from_slice(&16u16.to_le_bytes()); // Bit pro Sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round_ties_even() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zu_kurz() {
        assert!(is_too_short(0.39));
        assert!(!is_too_short(0.4));
    }

    #[test]
    fn resampling_laenge_und_werte() {
        let a: Vec<f32> = (0..48000).map(|i| (i % 7) as f32 / 10.0).collect();
        assert_eq!(resample_linear(&a, 48000, 16000).len(), 16000);
        assert_eq!(resample_linear(&a[..441], 44100, 16000).len(), 160);
        // Einfaches Beispiel per Hand: 4 -> 2 Werte nimmt jeden zweiten
        assert_eq!(resample_linear(&[0.0, 1.0, 2.0, 3.0], 4, 2), vec![0.0, 2.0]);
    }

    /// Vergleich mit `np.interp` (Testdaten erzeugt mit numpy).
    #[test]
    fn resampling_wie_numpy() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/resample_vectors.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for case in v.as_array().unwrap() {
            let f = |x: &serde_json::Value| -> Vec<f32> {
                x.as_array().unwrap().iter().map(|n| n.as_f64().unwrap() as f32).collect()
            };
            let got = resample_linear(&f(&case["input"]), case["rate"].as_u64().unwrap() as u32, 16000);
            let want = f(&case["output"]);
            assert_eq!(got.len(), want.len());
            let max_diff = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(max_diff < 1e-6, "Abweichung {max_diff}");
        }
    }

    #[test]
    fn wav_kopf_und_daten() {
        let d = std::env::temp_dir().join(format!("blitzvox-wav-{}.wav", std::process::id()));
        write_wav(&d, &[0.0, 1.0, -1.0, 0.5, 2.0], 16000).unwrap();
        let b = std::fs::read(&d).unwrap();
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(b.len(), 44 + 10);
        let samples: Vec<i16> = b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(samples, vec![0, 32767, -32767, 16384, 32767]);
        let _ = std::fs::remove_file(d);
    }

    /// Echte Aufnahme vom Standardmikrofon (1 Sekunde).
    #[test]
    #[ignore = "nimmt vom echten Mikrofon auf; manuell mit --ignored starten"]
    fn live_aufnahme() {
        for d in list_input_devices() {
            println!("Gerät: {} | Kanäle {} | Standard {}", d.name, d.channels, d.default);
        }
        let mut r = AudioRecorder::new(None);
        r.start();
        assert_eq!(r.error, None);
        let mut max_level = 0.0f32;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(50));
            max_level = max_level.max(r.audio_level());
        }
        let path = r.stop().expect("WAV-Datei");
        let len = std::fs::metadata(&path).unwrap().len();
        println!("Dauer {:.2} s, Datei {} Bytes, Eingangsrate {} Hz, max. Pegel {:.3}", r.duration(), len, r.device_rate, max_level);
        // Das Öffnen des Mikrofons dauert einen Moment, daher nicht die volle Sekunde
        assert!(len > 44 + 2 * 8000, "zu wenig Audiodaten");
        r.discard();
        assert!(!path.exists());
    }
}
