//! Ablauf der Workflows – Nachbau von `BlitzVox alt/blitzvox/workflow.py`.
//!
//! Jeder Workflow läuft gleich ab:
//!   1. Aufnahme starten
//!   2. Aufnahme stoppen
//!   3. Transkribieren (STT)
//!   4. Optional umschreiben (LLM)
//!   5. Ergebnis ausgeben (Zwischenablage/Einfügen übernimmt die Oberfläche)
//!
//! Abweichung von 1.0.8: Die temporäre WAV-Datei wird auch gelöscht, wenn die
//! Erkennung fehlschlägt (1.0.8 ließ sie dann im Temp-Ordner liegen).

// Wird erst mit der Oberfläche vollständig benutzt.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::corrections;
use crate::llm;
use crate::recorder::{AudioRecorder, is_too_short};
use crate::settings::{AppSettings, WorkflowModelConfig};
use crate::stt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowKind {
    /// Nur Transkription
    Transcription,
    /// Transkribieren -> verbessern
    TextImprove,
    /// Transkribieren -> beruhigen
    DampfAblassen,
    /// Transkribieren -> Emojis einfügen
    EmojiText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
    Rewriting,
    Done,
    Error,
}

impl Phase {
    /// Name wie der Python-Enum-Wert (für die Oberfläche).
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Recording => "recording",
            Phase::Transcribing => "transcribing",
            Phase::Rewriting => "rewriting",
            Phase::Done => "done",
            Phase::Error => "error",
        }
    }
}

/// (Phase, Meldung)
pub type PhaseCallback = Arc<dyn Fn(Phase, &str) + Send + Sync>;
/// Ergebnistext
pub type OutputCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// Gemeinsamer Zustand zwischen Runner und laufender Verarbeitung.
struct Shared {
    phase: Mutex<Phase>,
    /// Wird bei jedem Abbruch erhöht; eine Verarbeitung meldet nur, solange
    /// ihre Nummer noch aktuell ist.
    generation: AtomicU64,
    on_phase: Option<PhaseCallback>,
    on_output: Option<OutputCallback>,
}

impl Shared {
    fn set_phase(&self, run_id: u64, phase: Phase, message: &str) {
        if self.generation.load(Ordering::SeqCst) != run_id {
            return; // abgebrochen
        }
        *self.phase.lock().unwrap_or_else(|e| e.into_inner()) = phase;
        if let Some(cb) = &self.on_phase {
            cb(phase, message);
        }
    }

    fn output(&self, run_id: u64, text: &str) {
        if self.generation.load(Ordering::SeqCst) != run_id {
            return;
        }
        if let Some(cb) = &self.on_output {
            cb(text);
        }
    }
}

/// Verwaltet Aufnahme und Verarbeitung für einen Workflow.
pub struct WorkflowRunner {
    pub kind: WorkflowKind,
    settings: AppSettings,
    shared: Arc<Shared>,
    recorder: AudioRecorder,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl WorkflowRunner {
    pub fn new(
        kind: WorkflowKind,
        settings: AppSettings,
        on_phase: Option<PhaseCallback>,
        on_output: Option<OutputCallback>,
    ) -> Self {
        Self {
            kind,
            settings,
            shared: Arc::new(Shared {
                phase: Mutex::new(Phase::Idle),
                generation: AtomicU64::new(0),
                on_phase,
                on_output,
            }),
            recorder: AudioRecorder::new(None),
            task: None,
        }
    }

    /// Neue Einstellungen übernehmen (nach dem Speichern in den Einstellungen).
    pub fn set_settings(&mut self, settings: AppSettings) {
        self.settings = settings;
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn phase(&self) -> Phase {
        *self.shared.phase.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_recording()
    }

    pub fn audio_level(&self) -> f32 {
        self.recorder.audio_level()
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    fn set_phase(&self, phase: Phase, message: &str) {
        self.shared.set_phase(self.generation(), phase, message);
    }

    pub fn start_recording(&mut self) {
        if self.phase() != Phase::Idle {
            return;
        }
        // Recorder bei jedem Start frisch erzeugen – vermeidet Zustandsprobleme
        // aus der vorherigen Aufnahme.
        self.recorder = AudioRecorder::new(self.settings.audio_input_device_id.clone());
        self.recorder.start();
        match self.recorder.error.clone() {
            Some(e) => self.set_phase(Phase::Error, &e),
            None => self.set_phase(Phase::Recording, "Aufnahme läuft …"),
        }
    }

    pub fn stop_recording(&mut self) {
        if self.phase() != Phase::Recording {
            return;
        }
        let audio_path = self.recorder.stop();
        let duration = self.recorder.duration();
        let Some(audio_path) = audio_path else {
            let msg = self.recorder.error.clone().unwrap_or_else(|| "Aufnahme fehlgeschlagen.".into());
            self.set_phase(Phase::Error, &msg);
            return;
        };
        if is_too_short(duration) {
            let _ = std::fs::remove_file(&audio_path);
            self.set_phase(Phase::Error, "Aufnahme zu kurz — bitte länger sprechen.");
            return;
        }
        self.run_async(audio_path, duration);
    }

    /// Bricht Aufnahme bzw. Verarbeitung ab und kehrt in den Ruhezustand zurück.
    pub fn cancel(&mut self) {
        let next = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.recorder.discard();
        self.shared.set_phase(next, Phase::Idle, "");
    }

    pub fn reset(&mut self) {
        self.cancel();
    }

    fn run_async(&mut self, audio_path: PathBuf, duration: f64) {
        let shared = self.shared.clone();
        let settings = self.settings.clone();
        let kind = self.kind;
        let run_id = self.generation();
        self.task = Some(tauri::async_runtime::spawn(async move {
            let result = process(kind, &settings, &audio_path, duration, |p, m| shared.set_phase(run_id, p, m)).await;
            let _ = std::fs::remove_file(&audio_path);
            match result {
                Ok(text) => {
                    corrections::set_last(&text); // für „Letzten Text korrigieren“
                    shared.set_phase(run_id, Phase::Done, &text);
                    shared.output(run_id, &text);
                }
                Err(msg) => shared.set_phase(run_id, Phase::Error, &msg),
            }
        }));
    }
}

/// STT-Einstellungen des Workflows.
pub fn stt_config(kind: WorkflowKind, s: &AppSettings) -> &WorkflowModelConfig {
    match kind {
        WorkflowKind::Transcription => &s.transcription_model,
        WorkflowKind::TextImprove => &s.text_improvement.model,
        WorkflowKind::DampfAblassen => &s.dampf_ablassen.model,
        WorkflowKind::EmojiText => &s.emoji.model,
    }
}

/// Verarbeitung nach der Aufnahme (wie `_process`). Liefert den fertigen
/// Text oder die Fehlermeldung. `phase` meldet Zwischenschritte.
pub async fn process(
    kind: WorkflowKind,
    settings: &AppSettings,
    audio_path: &Path,
    duration: f64,
    phase: impl Fn(Phase, &str),
) -> Result<String, String> {
    // ── Schritt 1: Transkribieren ────────────────────────────────────────────
    phase(Phase::Transcribing, "Wird transkribiert …");
    let cfg = stt_config(kind, settings);
    // Hinweise an die Erkennung: Satzzeichen-Befehle + gelernte Wörter
    let stt_prompt = corrections::build_stt_prompt(settings.spoken_punctuation);
    let text = stt::transcribe(audio_path, cfg, &stt_prompt, &corrections::get_vocabulary())
        .await
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(audio_path);

    let text = stt::clean_transcript(&text);
    if stt::is_likely_artifact(&text, duration)
        || (!stt_prompt.is_empty() && corrections::is_prompt_echo(&text, &stt_prompt))
    {
        return Err("Keine Sprache erkannt.".into());
    }

    // ── Schritt 1b: Korrektur-Wörterbuch ─────────────────────────────────────
    let mut text = corrections::apply_replacements(&text);

    // Diktierte Satzzeichen-Befehle im Text?
    let mut punct = settings.spoken_punctuation && corrections::has_spoken_punctuation(&text);

    // Bleibt nach dem Umwandeln der Satzzeichen-Befehle kein Wort übrig, war es
    // keine echte Sprache (z. B. erkennt Groq bei Stille manchmal nur „Komma“,
    // das sonst als einzelnes „,“ eingefügt würde). Neu in 2.0.
    let words_left = if punct { corrections::apply_spoken_punctuation(&text) } else { text.clone() };
    if !has_words(&words_left) {
        log::info!("{kind:?}: nur Satzzeichen erkannt – verworfen");
        return Err("Keine Sprache erkannt.".into());
    }

    // ── Schritt 2: Sprachmodell ──────────────────────────────────────────────
    let llm_cfg = match kind {
        WorkflowKind::Transcription | WorkflowKind::TextImprove => &settings.text_improvement.model,
        WorkflowKind::DampfAblassen => &settings.dampf_ablassen.model,
        WorkflowKind::EmojiText => &settings.emoji.model,
    };
    log::info!(
        "{kind:?}: erkannt {} Zeichen, Sprachmodell {} / {:?}, Satzzeichen-Befehle: {punct}",
        text.chars().count(),
        llm_cfg.llm_provider,
        llm_cfg.llm_model
    );
    let input_len = text.chars().count();
    let e = |e: llm::LlmError| {
        log::warn!("{kind:?}: Sprachmodell-Fehler: {e}");
        e.to_string()
    };
    match kind {
        WorkflowKind::Transcription => {
            // Whisper setzt wegen des Hinweis-Prompts kaum eigene Zeichen ->
            // LLM setzt Satzzeichen/Großschreibung bei JEDER Transkription.
            if settings.spoken_punctuation {
                phase(Phase::Rewriting, "Satzzeichen werden gesetzt …");
                text = punctuate(&text, settings).await;
                punct = true; // Aufräumen der Zeichen unten aktivieren
            }
        }
        WorkflowKind::TextImprove => {
            phase(Phase::Rewriting, "Text wird verbessert …");
            text = llm::improve_text(&text, &settings.text_improvement, punct, settings).await.map_err(e)?;
        }
        WorkflowKind::DampfAblassen => {
            phase(Phase::Rewriting, "Wird umformuliert …");
            text = llm::dampf_ablassen(&text, &settings.dampf_ablassen, punct, settings).await.map_err(e)?;
        }
        WorkflowKind::EmojiText => {
            phase(Phase::Rewriting, "Emojis werden eingefügt …");
            text = llm::add_emojis(&text, &settings.emoji, punct, settings).await.map_err(e)?;
        }
    }

    if punct {
        text = corrections::tidy_punctuation(&text);
    }
    log::info!("{kind:?}: fertig, {} Zeichen (vorher {input_len})", text.chars().count());
    if !has_words(&text) {
        log::info!("{kind:?}: Ergebnis ohne Wörter – verworfen");
        return Err("Keine Sprache erkannt.".into());
    }
    Ok(text)
}

/// Enthält der Text mindestens einen Buchstaben oder eine Ziffer?
fn has_words(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// Satzzeichen per LLM (beachtet den Satzbau). Rückfall auf Regeln, wenn das
/// LLM nicht erreichbar ist oder mehr als die Befehle verändert hat.
async fn punctuate(text: &str, settings: &AppSettings) -> String {
    // Die Transkription hat keine eigene LLM-Auswahl -> Sprachmodell aus
    // „Text verbessern“ verwenden.
    let cfg = &settings.text_improvement.model;
    match llm::convert_punctuation(text, cfg, settings).await {
        Ok(out) => {
            let out = corrections::tidy_punctuation(&out);
            if corrections::llm_output_is_safe(text, &out) {
                return out;
            }
            log::info!("Satzzeichen: Antwort des Sprachmodells verworfen (Wörter geändert), Regeln greifen");
        }
        Err(e) => log::warn!("Satzzeichen: Sprachmodell nicht erreichbar ({e}), Regeln greifen"),
    }
    corrections::apply_spoken_punctuation(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_satzzeichen_ist_keine_sprache() {
        for t in ["Komma", "komma.", "Punkt", "Fragezeichen", "Komma Punkt"] {
            let out = corrections::apply_spoken_punctuation(t);
            assert!(!has_words(&out), "{t:?} -> {out:?} sollte ohne Wörter sein");
        }
        for t in ["Hallo Komma wie geht's", "Termin um 9", "Ja Punkt"] {
            let out = corrections::apply_spoken_punctuation(t);
            assert!(has_words(&out), "{t:?} -> {out:?} sollte Wörter enthalten");
        }
        assert!(!has_words(" , . "));
        assert!(has_words("😀 ok"));
    }

    fn copy_of_test_wav() -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sprache_de.wav");
        let dst = std::env::temp_dir().join(format!("blitzvox-wf-{}-{}.wav", std::process::id(), rand_suffix()));
        std::fs::copy(src, &dst).unwrap();
        dst
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn zuordnung_der_einstellungen() {
        let mut s = AppSettings::default();
        s.emoji.model.language = "en".into();
        assert_eq!(stt_config(WorkflowKind::EmojiText, &s).language, "en");
        assert_eq!(stt_config(WorkflowKind::Transcription, &s).language, "de");
    }

    #[test]
    fn abbrechen_setzt_ruhezustand_und_sperrt_alte_meldungen() {
        let log: Arc<Mutex<Vec<(Phase, String)>>> = Arc::default();
        let l = log.clone();
        let mut r = WorkflowRunner::new(
            WorkflowKind::Transcription,
            AppSettings::default(),
            Some(Arc::new(move |p, m| l.lock().unwrap().push((p, m.to_string())))),
            None,
        );
        let old_gen = r.generation();
        r.cancel();
        assert_eq!(r.phase(), Phase::Idle);
        // Eine Meldung der abgebrochenen Verarbeitung wird verworfen
        r.shared.set_phase(old_gen, Phase::Done, "zu spät");
        assert_eq!(r.phase(), Phase::Idle);
        assert_eq!(log.lock().unwrap().as_slice(), &[(Phase::Idle, String::new())]);
    }

    /// Ganzer Ablauf mit Groq (kostenlos) und dem gespeicherten Key.
    #[tokio::test]
    #[ignore = "ruft Groq mit dem echten Key auf; manuell mit --ignored starten"]
    async fn ablauf_live_groq() {
        let s = AppSettings::default();
        for kind in [WorkflowKind::Transcription, WorkflowKind::EmojiText, WorkflowKind::DampfAblassen] {
            let wav = copy_of_test_wav();
            let phases = Mutex::new(Vec::new());
            let t = process(kind, &s, &wav, 3.0, |p, _| phases.lock().unwrap().push(p.as_str())).await;
            println!("{kind:?}: {:?} -> {t:?}", phases.lock().unwrap());
            assert!(t.is_ok());
            assert!(!wav.exists(), "Audiodatei wurde nicht gelöscht");
        }
    }

    /// Echte Aufnahme über den Runner: 1 s Stille ergibt
    /// „Aufnahme zu kurz“ nicht, aber „Keine Sprache erkannt“ oder Text.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "nimmt vom Mikrofon auf und ruft Groq auf; manuell mit --ignored starten"]
    async fn runner_live() {
        let log: Arc<Mutex<Vec<(Phase, String)>>> = Arc::default();
        let l = log.clone();
        let mut r = WorkflowRunner::new(
            WorkflowKind::Transcription,
            AppSettings::default(),
            Some(Arc::new(move |p, m| l.lock().unwrap().push((p, m.to_string())))),
            None,
        );
        r.start_recording();
        assert_eq!(r.phase(), Phase::Recording);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        r.stop_recording();
        assert_eq!(r.phase(), Phase::Error); // zu kurz
        r.reset();
        r.start_recording();
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        r.stop_recording();
        for _ in 0..100 {
            if matches!(r.phase(), Phase::Done | Phase::Error) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!("{:?}", log.lock().unwrap());
        assert!(matches!(r.phase(), Phase::Done | Phase::Error));
    }
}
