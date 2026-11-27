//! Einstellungen – 1:1-Nachbau von `BlitzVox alt/blitzvox/services/settings.py`.
//!
//! Die Datei `settings.json` liegt im selben Ordner wie bei der Python-Version
//! und hat exakt dieselben Feldnamen, dieselbe Reihenfolge und dasselbe Format
//! (2 Leerzeichen Einrückung, UTF-8 ohne Escapes, unter Windows CRLF), damit
//! beide Versionen dieselbe Datei lesen und schreiben können.
//! API-Keys liegen nicht hier, sondern im Schlüsselbund (siehe `secrets.rs`).

// Wird erst mit den Workflows und der Oberfläche vollständig benutzt.
#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ── Pfade ────────────────────────────────────────────────────────────────────

/// Konfigurationsordner wie `_config_path()` in settings.py:
/// Windows `%APPDATA%\blitzvox`, sonst `$XDG_CONFIG_HOME/blitzvox`
/// bzw. `~/.config/blitzvox`. Der Ordner wird bei Bedarf angelegt.
pub fn config_dir() -> io::Result<PathBuf> {
    let base = if cfg!(windows) {
        match std::env::var_os("APPDATA") {
            Some(v) => PathBuf::from(v),
            None => home_dir(),
        }
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(v) => PathBuf::from(v),
            None => home_dir().join(".config"),
        }
    };
    let dir = base.join("blitzvox");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_default()
}

pub fn settings_path() -> io::Result<PathBuf> {
    Ok(config_dir()?.join("settings.json"))
}

// ── Anbieter ─────────────────────────────────────────────────────────────────

pub mod stt_provider {
    pub const GROQ: &str = "groq_whisper";
    pub const OPENAI: &str = "openai_whisper";
    pub const ASSEMBLYAI: &str = "assemblyai";
    pub const LOCAL: &str = "local_whisper";
    pub const OPENROUTER: &str = "openrouter_whisper";
    pub const CUSTOM: &str = "custom";

    pub const ALL: [&str; 6] = [GROQ, OPENAI, ASSEMBLYAI, LOCAL, OPENROUTER, CUSTOM];

    /// Anzeigenamen wie `STTProvider.DISPLAY`. LOCAL nennt statt
    /// „faster-whisper“ (1.0.8) jetzt whisper.cpp.
    pub fn display(id: &str) -> Option<&'static str> {
        Some(match id {
            GROQ => "Groq Whisper (kostenlos, Standard)",
            OPENAI => "OpenAI Whisper API",
            ASSEMBLYAI => "AssemblyAI",
            LOCAL => "Lokal – whisper.cpp (offline)",
            OPENROUTER => "OpenRouter (Whisper-Modelle)",
            CUSTOM => "Eigener Endpunkt",
            _ => return None,
        })
    }
}

pub mod llm_provider {
    pub const GROQ: &str = "groq";
    pub const OPENAI: &str = "openai";
    pub const ANTHROPIC: &str = "anthropic";
    pub const OLLAMA: &str = "ollama";
    pub const OPENROUTER: &str = "openrouter";
    pub const CUSTOM: &str = "custom";

    pub const ALL: [&str; 6] = [GROQ, OPENAI, ANTHROPIC, OLLAMA, OPENROUTER, CUSTOM];

    pub fn display(id: &str) -> Option<&'static str> {
        Some(match id {
            GROQ => "Groq (kostenlos, Standard)",
            OPENAI => "OpenAI",
            ANTHROPIC => "Claude (Anthropic)",
            OLLAMA => "Lokal – Ollama (eigenes Programm)",
            OPENROUTER => "OpenRouter (alle Modelle)",
            CUSTOM => "Eigener Endpunkt",
            _ => return None,
        })
    }

    /// Vorschlagslisten wie `LLM_MODEL_PRESETS` (Stand der Python-Version 1.0.8).
    pub fn model_presets(id: &str) -> &'static [&'static str] {
        match id {
            GROQ => &["openai/gpt-oss-120b", "openai/gpt-oss-20b", "qwen/qwen3.8-27b"],
            OPENAI => &[
                "gpt-5.6-luna",
                "gpt-5.6-terra",
                "gpt-5.6-sol",
                "gpt-6-astra",
                "gpt-4.1",
                "gpt-4o-mini",
            ],
            ANTHROPIC => &[
                "claude-sonnet-5",
                "claude-opus-5",
                "claude-haiku-4-5-20251001",
                "claude-fable-5-1",
            ],
            OLLAMA => &["llama3.1", "qwen3.6", "gpt-oss:20b", "gemma4", "mistral-nemo"],
            // OpenRouter: Liste wird live geladen; eigener Endpunkt: keine Vorschläge.
            _ => &[],
        }
    }
}

// ── Strukturen (Feldreihenfolge = Reihenfolge in settings.py) ────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowModelConfig {
    pub stt_provider: String,
    pub stt_model: String,
    pub llm_provider: String,
    pub llm_model: String,
    pub custom_stt_url: String,
    pub custom_llm_url: String,
    pub local_model_size: String,
    pub language: String,
}

impl Default for WorkflowModelConfig {
    fn default() -> Self {
        Self {
            stt_provider: stt_provider::GROQ.into(),
            stt_model: "whisper-large-v3-turbo".into(),
            llm_provider: llm_provider::GROQ.into(),
            llm_model: "openai/gpt-oss-120b".into(),
            custom_stt_url: String::new(),
            custom_llm_url: String::new(),
            local_model_size: "base".into(),
            language: "de".into(),
        }
    }
}

/// Standard-Anweisung für „Dampf ablassen“: Text aus 1.0.8 plus
/// [`DAMPF_KURZ_ZUSATZ`] (Entscheidung vom 24.11.2026).
pub const DAMPF_ABLASSEN_PROMPT: &str = "Du erhältst ein emotional gesprochenes Transkript. \
Erkenne zuerst das eigentliche Ziel, Anliegen und den wahren Frust der Person. \
Formuliere daraus eine klare, respektvolle und wirksame Nachricht, mit der die Person \
ihr Ziel eher erreicht. Bewahre relevante Fakten, konkrete Probleme, Grenzen, \
Erwartungen und die nötige Dringlichkeit. Entferne Beleidigungen, Drohungen, Sarkasmus, \
Unterstellungen und unnötige Eskalation. Wenn mehrere Vorwürfe genannt werden, verdichte \
sie auf die entscheidenden Kernpunkte. Der Ton soll ruhig, menschlich, bestimmt und \
lösungsorientiert sein. Gib NUR die fertige Nachricht zurück. \
Halte die Nachricht kurz: höchstens etwa so lang wie das Transkript, ohne zusätzliche \
Anrede, Einleitung, Grußformel oder Erklärungen, sofern sie nicht im Transkript vorkommen.";

/// Neu in 2.0: Zusatz gegen sehr lange Antworten neuerer Sprachmodelle
/// (z. B. machte Claude aus 55 Zeichen 386). Entscheidung vom 24.11.2026.
pub const DAMPF_KURZ_ZUSATZ: &str = "Halte die Nachricht kurz: höchstens etwa so lang wie \
das Transkript, ohne zusätzliche Anrede, Einleitung, Grußformel oder Erklärungen, sofern \
sie nicht im Transkript vorkommen.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DampfAblassenSettings {
    pub system_prompt: String,
    pub model: WorkflowModelConfig,
}

impl Default for DampfAblassenSettings {
    fn default() -> Self {
        Self { system_prompt: DAMPF_ABLASSEN_PROMPT.into(), model: Default::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextImprovementSettings {
    pub system_prompt: String,
    pub context: String,
    pub tone: String,
    pub custom_terms: Vec<String>,
    pub model: WorkflowModelConfig,
}

impl Default for TextImprovementSettings {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            context: String::new(),
            tone: "neutral".into(),
            custom_terms: Vec::new(),
            model: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmojiSettings {
    pub density: String,
    pub model: WorkflowModelConfig,
}

impl Default for EmojiSettings {
    fn default() -> Self {
        Self { density: "mittel".into(), model: Default::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub transcription: String,
    pub text_improvement: String,
    pub dampf_ablassen: String,
    pub emoji_text: String,
    /// Fenster „Letzten Text korrigieren“
    pub correction: String,
    pub transcription_mode: String,
    pub text_improvement_mode: String,
    pub dampf_ablassen_mode: String,
    pub emoji_text_mode: String,
    pub enabled: bool,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            transcription: "ctrl+alt+w".into(),
            text_improvement: "ctrl+alt+b".into(),
            dampf_ablassen: "ctrl+alt+f".into(),
            emoji_text: "ctrl+alt+j".into(),
            correction: "ctrl+alt+k".into(),
            transcription_mode: "hold".into(),
            text_improvement_mode: "toggle".into(),
            dampf_ablassen_mode: "hold".into(),
            emoji_text_mode: "hold".into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub hotkeys: HotkeyConfig,
    pub transcription_model: WorkflowModelConfig,
    pub text_improvement: TextImprovementSettings,
    pub dampf_ablassen: DampfAblassenSettings,
    pub emoji: EmojiSettings,
    pub launch_at_login: bool,
    pub auto_paste: bool,
    pub has_seen_onboarding: bool,
    pub groq_stt_model: String,
    pub groq_llm_model: String,
    pub anthropic_model: String,
    pub ollama_base_url: String,
    pub ollama_llm_model: String,
    pub openrouter_llm_model: String,
    pub custom_stt_url: String,
    pub custom_llm_url: String,
    /// Mikrofon-Nummer aus BlitzVox 1.0.8 (PortAudio). Wird von 2.0 nicht
    /// mehr benutzt, aber unverändert weitergeschrieben, damit 1.0.8 sie
    /// nach einem Zurückwechseln wieder findet.
    pub audio_input_device: Option<i64>,
    /// „Komma“ -> ","
    pub spoken_punctuation: bool,
    /// Neu in 2.0: gewähltes Mikrofon als stabile Geräte-ID (siehe
    /// `recorder::list_input_devices`). `None` = Standardgerät des Systems.
    /// Entscheidung vom 24.11.2026, weil sich die alten PortAudio-Nummern
    /// keinem Gerät zuordnen lassen. 1.0.8 ignoriert dieses Feld.
    pub audio_input_device_id: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkeys: Default::default(),
            transcription_model: Default::default(),
            text_improvement: Default::default(),
            dampf_ablassen: Default::default(),
            emoji: Default::default(),
            launch_at_login: false,
            auto_paste: true,
            has_seen_onboarding: false,
            groq_stt_model: "whisper-large-v3-turbo".into(),
            groq_llm_model: "openai/gpt-oss-120b".into(),
            anthropic_model: "claude-sonnet-5".into(),
            ollama_base_url: "http://localhost:11434".into(),
            ollama_llm_model: "llama3.2".into(),
            openrouter_llm_model: String::new(),
            custom_stt_url: String::new(),
            custom_llm_url: String::new(),
            audio_input_device: None,
            spoken_punctuation: true,
            audio_input_device_id: None,
        }
    }
}

// ── Laden / Speichern ────────────────────────────────────────────────────────

/// Wie `_nested_update`: Werte aus `source` überschreiben `target`,
/// verschachtelte Objekte werden zusammengeführt statt ersetzt.
fn nested_update(target: &mut Map<String, Value>, source: Map<String, Value>) {
    for (k, v) in source {
        match (target.get_mut(&k), v) {
            (Some(Value::Object(t)), Value::Object(s)) => nested_update(t, s),
            (_, v) => {
                target.insert(k, v);
            }
        }
    }
}

static LOAD_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Fehlermeldung, falls die Einstellungen beim letzten Laden nicht lesbar waren.
pub fn get_load_error() -> Option<String> {
    LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set_load_error(v: Option<String>) {
    *LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

/// Liest gespeicherte Werte, füllt fehlende mit Standardwerten auf und
/// ignoriert unbekannte Felder (fallen beim nächsten Speichern weg).
pub(crate) fn parse_merged(text: &str) -> Result<AppSettings, String> {
    let saved: Value = serde_json::from_str(text).map_err(|e| format!("JSONDecodeError: {e}"))?;
    let Value::Object(saved) = saved else {
        return Err("Fehler: Die Datei enthält kein JSON-Objekt.".into());
    };
    let Value::Object(mut merged) =
        serde_json::to_value(AppSettings::default()).map_err(|e| e.to_string())?
    else {
        unreachable!("AppSettings wird immer als Objekt serialisiert");
    };
    nested_update(&mut merged, saved);
    serde_json::from_value(Value::Object(merged)).map_err(|e| format!("Ungültiger Wert: {e}"))
}

/// Lädt die Einstellungen aus `path`. Ist die Datei kaputt, wird sie als
/// `settings.defekt.json` gesichert und neben der Standardeinstellung die
/// Fehlermeldung zurückgegeben (wie `load_settings()` in settings.py).
pub fn load_from(path: &Path) -> (AppSettings, Option<String>) {
    if !path.exists() {
        return (AppSettings::default(), None);
    }
    let result = fs::read(path)
        .map_err(|e| format!("OSError: {e}"))
        .and_then(|b| String::from_utf8(b).map_err(|e| format!("UnicodeDecodeError: {e}")))
        .and_then(|t| parse_merged(&t));
    match result {
        Ok(s) => (s, None),
        Err(err) => {
            let backup = path.with_file_name("settings.defekt.json");
            let backup_info = match fs::copy(path, &backup) {
                Ok(_) => format!("Gesichert unter: {}", backup.display()),
                Err(e) => format!("Sicherung fehlgeschlagen: {e}"),
            };
            let msg = format!("{}
{}
{}", path.display(), err, backup_info);
            (AppSettings::default(), Some(msg))
        }
    }
}

/// Lädt `settings.json` aus dem Konfigurationsordner; eine Fehlermeldung
/// ist danach über `get_load_error()` abrufbar.
pub fn load_settings() -> AppSettings {
    let (s, err) = match settings_path() {
        Ok(p) => load_from(&p),
        Err(e) => (AppSettings::default(), Some(format!("Konfigurationsordner nicht verfügbar: {e}"))),
    };
    set_load_error(err);
    s
}

/// JSON wie Pythons `json.dump(..., indent=2, ensure_ascii=False)`;
/// unter Windows mit CRLF, weil Python dort im Textmodus schreibt.
pub(crate) fn to_python_json<T: Serialize>(value: &T) -> io::Result<String> {
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    // Zeilenumbrüche in Texten sind im JSON als \n maskiert, echte Umbrüche
    // gibt es also nur zwischen den Zeilen – die Umwandlung ist sicher.
    Ok(if cfg!(windows) { json.replace('\n', "\r\n") } else { json })
}

pub fn save_to(path: &Path, s: &AppSettings) -> io::Result<()> {
    fs::write(path, to_python_json(s)?)
}

pub fn save_settings(s: &AppSettings) -> io::Result<()> {
    save_to(&settings_path()?, s)
}

// ── Hilfsfunktionen ──────────────────────────────────────────────────────────

/// Maskierter Key für die Anzeige, wie `masked()` in settings.py.
pub fn masked(key: Option<&str>) -> String {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return "—".into();
    };
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}••••••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("blitzvox-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fehlende_felder_werden_aufgefuellt_unbekannte_ignoriert() {
        let s = parse_merged(
            r#"{"auto_paste": false, "hotkeys": {"transcription": "ctrl+alt+x"},
                "emoji": {"model": {"language": "en"}}, "gibt_es_nicht": 1}"#,
        )
        .unwrap();
        assert!(!s.auto_paste);
        assert_eq!(s.hotkeys.transcription, "ctrl+alt+x");
        assert_eq!(s.hotkeys.text_improvement, "ctrl+alt+b");
        assert_eq!(s.emoji.model.language, "en");
        assert_eq!(s.emoji.model.stt_model, "whisper-large-v3-turbo");
        assert_eq!(s.emoji.density, "mittel");
    }

    #[test]
    fn defekte_datei_wird_gesichert() {
        let d = temp_dir("defekt");
        let p = d.join("settings.json");
        fs::write(&p, "{ kaputt").unwrap();
        let (s, err) = load_from(&p);
        assert_eq!(s, AppSettings::default());
        assert!(d.join("settings.defekt.json").exists());
        assert!(err.unwrap().contains("Gesichert unter"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn standard_speichern_und_laden() {
        let d = temp_dir("rundlauf");
        let p = d.join("settings.json");
        let mut s = AppSettings::default();
        s.audio_input_device = Some(3); // alter Wert bleibt erhalten
        s.audio_input_device_id = Some("{0.0.1.00000000}.{abc}".into());
        s.text_improvement.custom_terms = vec!["BlitzVox".into(), "Über".into()];
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), (s, None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn maskierung() {
        assert_eq!(masked(None), "—");
        assert_eq!(masked(Some("")), "—");
        assert_eq!(masked(Some("12345678")), "••••••••");
        assert_eq!(masked(Some("gsk_abcdefghWXYZ")), "gsk_••••••••WXYZ");
    }

    /// Liest eine Kopie der echten settings.json der Python-Version, speichert
    /// sie wieder und prüft, dass Byte für Byte dasselbe herauskommt – bis auf
    /// das in 2.0 neue Feld `audio_input_device_id` am Ende.
    /// Die echte Datei wird nur gelesen. Fehlt sie, wird der Test übersprungen.
    #[test]
    fn echte_datei_bleibt_byte_gleich() {
        let Ok(real) = settings_path() else { return };
        let Ok(original) = fs::read(&real) else { return };
        let original = String::from_utf8(original).unwrap();
        let d = temp_dir("echt");
        let copy = d.join("settings.json");
        fs::write(&copy, &original).unwrap();
        let (s, err) = load_from(&copy);
        assert_eq!(err, None);
        let out = d.join("neu.json");
        save_to(&out, &s).unwrap();
        let neu = fs::read_to_string(&out).unwrap();
        let nl = if cfg!(windows) { "\r\n" } else { "\n" };
        let erwartet = if original.contains("\"audio_input_device_id\"") {
            original.clone()
        } else {
            let ende = format!("{nl}}}");
            let rumpf = original.strip_suffix(&ende).expect("Datei endet mit }");
            let wert = match &s.audio_input_device_id {
                Some(id) => serde_json::to_string(id).unwrap(),
                None => "null".into(),
            };
            format!("{rumpf},{nl}  \"audio_input_device_id\": {wert}{ende}")
        };
        assert!(neu == erwartet, "Ausgabe weicht von der Python-Datei ab");
        let _ = fs::remove_dir_all(&d);
    }
}
