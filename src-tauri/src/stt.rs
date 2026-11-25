//! Spracherkennung – Nachbau von `BlitzVox alt/blitzvox/services/stt.py`.
//!
//! Anbieter: Groq Whisper, OpenAI Whisper, AssemblyAI, OpenRouter, eigener
//! OpenAI-kompatibler Endpunkt. Die lokale Erkennung (bisher faster-whisper,
//! künftig whisper.cpp) folgt in einem eigenen Schritt.

// Wird erst mit den Workflows vollständig benutzt.
#![allow(dead_code)]

use std::fmt;
use std::path::Path;
use std::time::Duration;

use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::http::{client, error_message, request_error};
use crate::secrets::{
    KEY_ASSEMBLYAI, KEY_CUSTOM_STT, KEY_GROQ, KEY_OPENAI, KEY_OPENROUTER, get_api_key,
};
use crate::settings::{WorkflowModelConfig, stt_provider};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttError(pub String);

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SttError {}

type Result<T> = std::result::Result<T, SttError>;

fn err(msg: impl Into<String>) -> SttError {
    SttError(msg.into())
}

const OPENAI_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const GROQ_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const ASSEMBLYAI_BASE: &str = "https://api.assemblyai.com";
pub const OPENROUTER_STT_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";

/// Leitet an den gewählten Anbieter weiter.
/// `prompt`: Hinweistext für Whisper (OpenAI, Groq, eigener Endpunkt, lokal);
/// AssemblyAI und OpenRouter ignorieren ihn.
/// `keyterms`: gelernte Schreibweisen; nur AssemblyAI.
pub async fn transcribe(
    audio_path: &Path,
    cfg: &WorkflowModelConfig,
    prompt: &str,
    keyterms: &[String],
) -> Result<String> {
    match cfg.stt_provider.as_str() {
        stt_provider::OPENAI => {
            let key = get_api_key(KEY_OPENAI)
                .ok_or_else(|| err("OpenAI API Key fehlt. Bitte in den Einstellungen hinterlegen."))?;
            let model = non_empty(&cfg.stt_model).unwrap_or("whisper-1");
            whisper_api("OpenAI", OPENAI_URL, &key, model, &cfg.language, prompt, audio_path).await
        }
        stt_provider::GROQ => {
            let key = get_api_key(KEY_GROQ)
                .ok_or_else(|| err("Groq API Key fehlt. Bitte in den Einstellungen hinterlegen."))?;
            let model = non_empty(&cfg.stt_model).unwrap_or("whisper-large-v3-turbo");
            whisper_api("Groq", GROQ_URL, &key, model, &cfg.language, prompt, audio_path).await
        }
        stt_provider::ASSEMBLYAI => {
            let key = get_api_key(KEY_ASSEMBLYAI).ok_or_else(|| {
                err("AssemblyAI API Key fehlt. Bitte in den Einstellungen hinterlegen.")
            })?;
            assemblyai(ASSEMBLYAI_BASE, &key, &cfg.language, keyterms, audio_path, Duration::from_secs(1))
                .await
        }
        stt_provider::LOCAL => {
            let size = non_empty(&cfg.local_model_size).unwrap_or("base").to_string();
            // Modell bei Bedarf herunterladen (Fortschritt zeigt später die Oberfläche
            // über local_stt::ensure_model an, bevor die Erkennung startet)
            crate::local_stt::ensure_model(&size, |_, _| {}).await.map_err(err)?;
            let (path, lang, prompt) = (audio_path.to_path_buf(), cfg.language.clone(), prompt.to_string());
            tokio::task::spawn_blocking(move || {
                crate::local_stt::transcribe_file(&path, &size, &lang, &prompt)
            })
            .await
            .map_err(|e| err(e.to_string()))?
            .map_err(err)
        }
        stt_provider::OPENROUTER => {
            let key = get_api_key(KEY_OPENROUTER).ok_or_else(|| {
                err("OpenRouter API Key fehlt. Bitte in den Einstellungen hinterlegen.")
            })?;
            let model = non_empty(cfg.stt_model.trim()).unwrap_or("openai/whisper-1");
            openrouter(OPENROUTER_STT_URL, &key, model, &cfg.language, audio_path).await
        }
        stt_provider::CUSTOM => {
            if cfg.custom_stt_url.is_empty() {
                return Err(err("Kein eigener STT-Endpunkt konfiguriert."));
            }
            let key = get_api_key(KEY_CUSTOM_STT);
            let model = non_empty(&cfg.stt_model).unwrap_or("whisper-1");
            custom(&cfg.custom_stt_url, key.as_deref(), model, &cfg.language, prompt, audio_path).await
        }
        other => Err(err(format!("Unbekannter STT-Anbieter: {other}"))),
    }
}

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

fn read_audio(path: &Path) -> Result<(String, Vec<u8>)> {
    let bytes = std::fs::read(path).map_err(|e| err(e.to_string()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((name, bytes))
}

fn audio_part(name: &str, bytes: Vec<u8>) -> Result<Part> {
    Part::bytes(bytes)
        .file_name(name.to_string())
        .mime_str("audio/wav")
        .map_err(|e| err(e.to_string()))
}

/// Fehler wie `_raise_api_error(provider, resp)`.
fn api_error(provider: &str, status: u16, body: &str) -> SttError {
    err(format!("{provider} Fehler {status}: {}", error_message(body)))
}

async fn send(req: reqwest::RequestBuilder) -> Result<(u16, String)> {
    let resp = req.send().await.map_err(|e| err(request_error(e)))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| err(request_error(e)))?;
    Ok((status, body))
}

/// OpenAI und Groq: gleiche Whisper-Schnittstelle, Antwort als reiner Text.
async fn whisper_api(
    provider: &str,
    url: &str,
    api_key: &str,
    model: &str,
    language: &str,
    prompt: &str,
    audio_path: &Path,
) -> Result<String> {
    let (name, bytes) = read_audio(audio_path)?;
    let mut form = Form::new()
        .text("model", model.to_string())
        .text("response_format", "text")
        .part("file", audio_part(&name, bytes)?);
    if !language.is_empty() {
        form = form.text("language", language.to_string());
    }
    if !prompt.is_empty() {
        form = form.text("prompt", prompt.to_string());
    }
    let http = client(60).map_err(err)?;
    let (status, body) = send(http.post(url).bearer_auth(api_key).multipart(form)).await?;
    if status != 200 {
        return Err(api_error(provider, status, &body));
    }
    Ok(body.trim().to_string())
}

/// AssemblyAI: Datei hochladen, Auftrag anlegen, Ergebnis abfragen.
async fn assemblyai(
    base: &str,
    api_key: &str,
    language: &str,
    keyterms: &[String],
    audio_path: &Path,
    poll_interval: Duration,
) -> Result<String> {
    let http = client(120).map_err(err)?;
    let (_, bytes) = read_audio(audio_path)?;

    // 1. Audio hochladen
    let (status, body) =
        send(http.post(format!("{base}/v2/upload")).header("authorization", api_key).body(bytes)).await?;
    if status != 200 {
        return Err(err(format!("AssemblyAI Upload fehlgeschlagen: {status}")));
    }
    let upload: Value = serde_json::from_str(&body).map_err(|e| err(e.to_string()))?;
    let audio_url = upload["upload_url"]
        .as_str()
        .ok_or_else(|| err("AssemblyAI: keine upload_url erhalten."))?
        .to_string();

    // 2. Transkription anfordern
    let mut payload = json!({ "audio_url": audio_url });
    if !language.is_empty() {
        payload["language_code"] = json!(language);
    } else {
        // Ohne language_code transkribiert AssemblyAI immer Englisch (en_us).
        // Für „Automatisch erkennen“ muss die Erkennung ausdrücklich an sein.
        // Beides zusammen ist nicht erlaubt (Fehler 400).
        payload["language_detection"] = json!(true);
    }
    if !keyterms.is_empty() {
        // AssemblyAI erlaubt max. 6 Wörter pro Begriff
        let terms: Vec<&String> =
            keyterms.iter().filter(|k| k.split_whitespace().count() <= 6).take(1000).collect();
        payload["keyterms_prompt"] = json!(terms);
    }
    let transcript_url = format!("{base}/v2/transcript");
    let post = |p: &Value| {
        http.post(&transcript_url)
            .header("authorization", api_key)
            .header("content-type", "application/json")
            .body(p.to_string())
    };
    let (mut status, mut body) = send(post(&payload)).await?;
    if status != 200 && payload.get("keyterms_prompt").is_some() {
        // Modell/Sprache unterstützt keyterms nicht -> ohne erneut versuchen
        payload.as_object_mut().map(|o| o.shift_remove("keyterms_prompt"));
        (status, body) = send(post(&payload)).await?;
    }
    if status != 200 {
        return Err(api_error("AssemblyAI", status, &body));
    }
    let created: Value = serde_json::from_str(&body).map_err(|e| err(e.to_string()))?;
    let id = match &created["id"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };

    // 3. Abfragen
    for _ in 0..120 {
        tokio::time::sleep(poll_interval).await;
        let (_, body) = send(
            http.get(format!("{transcript_url}/{id}"))
                .header("authorization", api_key)
                .header("content-type", "application/json"),
        )
        .await?;
        let poll: Value = serde_json::from_str(&body).map_err(|e| err(e.to_string()))?;
        match poll["status"].as_str() {
            Some("completed") => {
                return Ok(poll["text"].as_str().unwrap_or("").trim().to_string());
            }
            Some("error") => {
                let msg = match &poll["error"] {
                    Value::String(s) => s.clone(),
                    Value::Null => "None".into(),
                    other => other.to_string(),
                };
                return Err(err(format!("AssemblyAI Fehler: {msg}")));
            }
            _ => {}
        }
    }
    Err(err("AssemblyAI Timeout."))
}

/// OpenRouter: Antwort als JSON mit Feld `text`.
async fn openrouter(url: &str, api_key: &str, model: &str, language: &str, audio_path: &Path) -> Result<String> {
    let (name, bytes) = read_audio(audio_path)?;
    let mut form = Form::new().text("model", model.to_string()).part("file", audio_part(&name, bytes)?);
    if !language.is_empty() {
        form = form.text("language", language.to_string());
    }
    let http = client(60).map_err(err)?;
    let (status, body) = send(http.post(url).bearer_auth(api_key).multipart(form)).await?;
    if status != 200 {
        return Err(api_error("OpenRouter", status, &body));
    }
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["text"].as_str().map(|t| t.trim().to_string()))
        .ok_or_else(|| err("Keine Transkription von OpenRouter erhalten."))
}

/// Eigener Endpunkt (OpenAI-kompatibel). Kennt er „prompt“ nicht, wird ohne
/// erneut gesendet.
async fn custom(
    url: &str,
    api_key: Option<&str>,
    model: &str,
    language: &str,
    prompt: &str,
    audio_path: &Path,
) -> Result<String> {
    let (name, bytes) = read_audio(audio_path)?;
    let http = client(60).map_err(err)?;
    let form = |with_prompt: bool| -> Result<Form> {
        let mut f = Form::new()
            .text("model", model.to_string())
            .text("response_format", "text")
            .part("file", audio_part(&name, bytes.clone())?);
        if !language.is_empty() {
            f = f.text("language", language.to_string());
        }
        if with_prompt {
            f = f.text("prompt", prompt.to_string());
        }
        Ok(f)
    };
    let request = |f: Form| {
        let mut r = http.post(url).multipart(f);
        if let Some(k) = api_key {
            r = r.bearer_auth(k);
        }
        r
    };
    let with_prompt = !prompt.is_empty();
    let (mut status, mut body) = send(request(form(with_prompt)?)).await?;
    if status != 200 && with_prompt {
        (status, body) = send(request(form(false)?)).await?;
    }
    if status != 200 {
        return Err(api_error("Custom STT", status, &body));
    }
    Ok(body.trim().to_string())
}

// ── Modellliste von OpenRouter ───────────────────────────────────────────────

/// Alle Speech-to-Text-Modelle von OpenRouter (sortiert), bei Fehlern leer –
/// wie `fetch_openrouter_transcription_models()` in settings.py.
pub async fn fetch_openrouter_transcription_models() -> Vec<String> {
    crate::llm::fetch_model_ids(
        crate::llm::OPENROUTER_MODELS_URL,
        &[("output_modalities", "transcription")],
        get_api_key(KEY_OPENROUTER).as_deref(),
    )
    .await
}

// ── Bereinigung ──────────────────────────────────────────────────────────────

/// Typische Whisper-Fehlausgaben bei Stille.
const HALLUCINATIONS: [&str; 7] = [
    "Vielen Dank für Ihre Aufmerksamkeit.",
    "Danke für Ihre Aufmerksamkeit.",
    "Untertitel der Amara.org-Community",
    "Untertitel im Auftrag des ZDF",
    "Thank you for watching.",
    "Thanks for watching.",
    "Untertitelung des ZDF, 2020",
];

/// Entfernt typische Whisper-Fehlausgaben.
pub fn clean_transcript(text: &str) -> String {
    let t = text.trim();
    if HALLUCINATIONS.contains(&t) { String::new() } else { t.to_string() }
}

pub fn is_likely_artifact(text: &str, duration: f64) -> bool {
    let cleaned = clean_transcript(text);
    if cleaned.is_empty() {
        return true;
    }
    duration < 1.0 && cleaned.split_whitespace().count() < 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server;

    fn wav() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("blitzvox-stt-{}.wav", std::process::id()));
        crate::recorder::write_wav(&p, &[0.0; 160], 16000).unwrap();
        p
    }

    fn multipart_has(body: &str, field: &str, value: &str) -> bool {
        body.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n"))
    }

    #[test]
    fn bereinigung() {
        assert_eq!(clean_transcript("  Thanks for watching. "), "");
        assert_eq!(clean_transcript(" Hallo "), "Hallo");
        assert!(is_likely_artifact("Untertitel im Auftrag des ZDF", 5.0));
        assert!(is_likely_artifact("Hallo", 0.8));
        assert!(!is_likely_artifact("Hallo Welt", 0.8));
        assert!(!is_likely_artifact("Hallo", 1.0));
    }

    #[tokio::test]
    async fn whisper_anfrage_wie_python() {
        let srv = test_server::start(vec![(200, "  Hallo Welt\n")]).await;
        let url = format!("{}/v1/audio/transcriptions", srv.url);
        let t = whisper_api("Groq", &url, "KEY", "whisper-large-v3-turbo", "de", "Hinweis", &wav())
            .await
            .unwrap();
        assert_eq!(t, "Hallo Welt");
        let req = srv.requests.lock().unwrap()[0].clone();
        assert_eq!(req.method, "POST");
        assert_eq!(req.header("authorization"), Some("Bearer KEY"));
        let body = req.body_text();
        assert!(multipart_has(&body, "model", "whisper-large-v3-turbo"));
        assert!(multipart_has(&body, "response_format", "text"));
        assert!(multipart_has(&body, "language", "de"));
        assert!(multipart_has(&body, "prompt", "Hinweis"));
        assert!(body.contains("Content-Type: audio/wav"));
    }

    #[tokio::test]
    async fn whisper_ohne_sprache_und_prompt_und_fehler() {
        let srv = test_server::start(vec![(401, r#"{"error":{"message":"Invalid API Key"}}"#)]).await;
        let e = whisper_api("Groq", &srv.url, "K", "m", "", "", &wav()).await.unwrap_err();
        assert_eq!(e.0, "Groq Fehler 401: Invalid API Key");
        let body = srv.requests.lock().unwrap()[0].body_text();
        assert!(!body.contains("name=\"language\""));
        assert!(!body.contains("name=\"prompt\""));
    }

    #[tokio::test]
    async fn assemblyai_ablauf_mit_keyterm_wiederholung() {
        let srv = test_server::start(vec![
            (200, r#"{"upload_url":"https://cdn/x"}"#),
            (400, r#"{"error":"keyterms not supported"}"#),
            (200, r#"{"id":"abc"}"#),
            (200, r#"{"status":"processing"}"#),
            (200, r#"{"status":"completed","text":" Fertig. "}"#),
        ])
        .await;
        let terms = vec!["SCSI".to_string(), "eins zwei drei vier fünf sechs sieben".to_string()];
        let t = assemblyai(&srv.url, "AK", "", &terms, &wav(), Duration::from_millis(1)).await.unwrap();
        assert_eq!(t, "Fertig.");
        let reqs = srv.requests.lock().unwrap().clone();
        assert_eq!(reqs[0].path, "/v2/upload");
        assert_eq!(reqs[0].header("authorization"), Some("AK"));
        let first = reqs[1].json();
        assert_eq!(first["language_detection"], json!(true));
        assert_eq!(first["keyterms_prompt"], json!(["SCSI"])); // >6 Wörter aussortiert
        assert!(reqs[2].json().get("keyterms_prompt").is_none());
        assert_eq!(reqs[4].path, "/v2/transcript/abc");
        assert_eq!(reqs[4].method, "GET");
    }

    #[tokio::test]
    async fn assemblyai_fehlerstatus() {
        let srv = test_server::start(vec![
            (200, r#"{"upload_url":"u"}"#),
            (200, r#"{"id":"x"}"#),
            (200, r#"{"status":"error","error":"Audio zu kurz"}"#),
        ])
        .await;
        let e = assemblyai(&srv.url, "K", "de", &[], &wav(), Duration::from_millis(1)).await.unwrap_err();
        assert_eq!(e.0, "AssemblyAI Fehler: Audio zu kurz");
        let reqs = srv.requests.lock().unwrap().clone();
        assert_eq!(reqs[1].json()["language_code"], json!("de"));
        assert!(reqs[1].json().get("keyterms_prompt").is_none());
    }

    #[tokio::test]
    async fn openrouter_json_antwort() {
        let srv = test_server::start(vec![(200, r#"{"text":" Hallo "}"#)]).await;
        let t = openrouter(&srv.url, "K", "openai/whisper-1", "de", &wav()).await.unwrap();
        assert_eq!(t, "Hallo");
        let body = srv.requests.lock().unwrap()[0].body_text();
        assert!(!body.contains("response_format"));
        let srv = test_server::start(vec![(200, r#"{"other":1}"#)]).await;
        let e = openrouter(&srv.url, "K", "m", "", &wav()).await.unwrap_err();
        assert_eq!(e.0, "Keine Transkription von OpenRouter erhalten.");
    }

    #[tokio::test]
    async fn eigener_endpunkt_ohne_prompt_wiederholen() {
        let srv = test_server::start(vec![(422, "unknown field prompt"), (200, "ok ")]).await;
        let t = custom(&srv.url, None, "whisper-1", "", "Hinweis", &wav()).await.unwrap();
        assert_eq!(t, "ok");
        let reqs = srv.requests.lock().unwrap().clone();
        assert!(reqs[0].body_text().contains("name=\"prompt\""));
        assert!(!reqs[1].body_text().contains("name=\"prompt\""));
        assert_eq!(reqs[0].header("authorization"), None);
    }

    /// Echter Aufruf bei Groq mit dem gespeicherten Key (kostenloser Dienst).
    /// Erwartet die Datei aus `tests/data/sprache_de.wav`.
    #[tokio::test]
    #[ignore = "ruft Groq mit dem echten Key auf; manuell mit --ignored starten"]
    async fn groq_live() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sprache_de.wav");
        let cfg = WorkflowModelConfig::default();
        let t = transcribe(&path, &cfg, "", &[]).await.unwrap();
        println!("Groq erkannt: {t}");
        assert!(!t.is_empty());
    }
}
