//! Sprachmodelle für die Text-Workflows – Nachbau von
//! `BlitzVox alt/blitzvox/services/llm.py`.
//!
//! Anbieter: Groq, OpenAI, Claude (Anthropic), Ollama, OpenRouter, eigener
//! OpenAI-kompatibler Endpunkt.

// Wird erst mit den Workflows vollständig benutzt.
#![allow(dead_code)]

use std::fmt;

use serde_json::{Value, json};

use crate::http::{client, error_message, request_error};
use crate::secrets::{
    KEY_ANTHROPIC, KEY_CUSTOM_LLM, KEY_GROQ, KEY_OPENAI, KEY_OPENROUTER, get_api_key,
};
use crate::settings::{
    AppSettings, DampfAblassenSettings, EmojiSettings, TextImprovementSettings, WorkflowModelConfig,
    llm_provider,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmError(pub String);

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LlmError {}

type Result<T> = std::result::Result<T, LlmError>;

fn err(msg: impl Into<String>) -> LlmError {
    LlmError(msg.into())
}

const GROQ_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
pub const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
pub const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

// ── Öffentliche Funktionen ───────────────────────────────────────────────────

/// `app`: aktuelle Einstellungen (Python las sie hier jeweils neu von der
/// Platte; gebraucht werden Anthropic-/Ollama-/OpenRouter-Felder).
pub async fn improve_text(
    text: &str,
    settings: &TextImprovementSettings,
    punctuation: bool,
    app: &AppSettings,
) -> Result<String> {
    let mut system = build_improve_prompt(settings);
    if punctuation {
        system.push_str(&punctuation_hint());
    }
    complete(text, &system, &settings.model, 0.3, app).await
}

pub async fn dampf_ablassen(
    text: &str,
    settings: &DampfAblassenSettings,
    punctuation: bool,
    app: &AppSettings,
) -> Result<String> {
    let mut system = settings.system_prompt.clone();
    if punctuation {
        system.push_str(&punctuation_hint());
    }
    complete(text, &system, &settings.model, 0.4, app).await
}

pub async fn add_emojis(
    text: &str,
    settings: &EmojiSettings,
    punctuation: bool,
    app: &AppSettings,
) -> Result<String> {
    let mut system = build_emoji_prompt(&settings.density);
    if punctuation {
        system.push_str(&punctuation_hint());
    }
    complete(text, &system, &settings.model, 0.3, app).await
}

/// Nur diktierte Satzzeichen-Befehle umsetzen, sonst nichts ändern.
pub async fn convert_punctuation(text: &str, cfg: &WorkflowModelConfig, app: &AppSettings) -> Result<String> {
    complete(text, &punctuation_prompt(), cfg, 0.0, app).await
}

// ── Diktierte Satzzeichen ────────────────────────────────────────────────────

const PUNCTUATION_RULES: &str = "Das Transkript kann gesprochene Satzzeichen-Befehle enthalten: \
Komma (,), Punkt (.), Fragezeichen (?), Ausrufezeichen (!), Doppelpunkt (:), \
Semikolon oder Strichpunkt (;), Bindestrich (-), Gedankenstrich (–), \
Auslassungspunkte (…), Klammer auf ((), Klammer zu ()), \
Anführungszeichen auf oder unten („), Anführungszeichen zu oder oben (“), \
neue Zeile (Zeilenumbruch), neuer Absatz (Leerzeile).\n\
Entscheide anhand von Satzbau und Grammatik, ob ein solches Wort als Befehl \
gemeint ist oder als normales Wort im Satz. Beispiele für normale Wörter, \
die stehen bleiben: \"Mir fehlt das Komma\", \"ein wichtiger Punkt\", \
\"Punkt 12 Uhr\", \"eine neue Zeile im Gedicht\". \
Ersetze nur echte Befehle durch das Zeichen. Entferne dabei automatisch \
gesetzte Satzzeichen direkt am Befehl, damit nichts doppelt steht.";

pub fn punctuation_hint() -> String {
    format!("\n\n{PUNCTUATION_RULES}")
}

pub fn punctuation_prompt() -> String {
    format!(
        "Du erhältst ein diktiertes Transkript, oft ganz ohne Satzzeichen.\n\
{PUNCTUATION_RULES}\n\
Setze außerdem überall die fehlenden Satzzeichen nach den Regeln der deutschen \
Rechtschreibung (Punkt, Komma, Fragezeichen usw.) und korrigiere die \
Groß- und Kleinschreibung (Satzanfänge, Nomen, nominalisierte Wörter wie \
\"etwas Leckeres\").\n\
Setze Bindestriche in Zusammensetzungen mit Abkürzungen, Buchstaben oder \
Ziffern (z. B. SCSI-Festplatte, USB-Stick, E-Mail, 3-Zimmer-Wohnung).\n\
Ändere sonst NICHTS: keine Wörter hinzufügen, entfernen, umstellen, ersetzen \
oder umformulieren, auch keine Umgangssprache glätten. \
Gib NUR den Text zurück, ohne Anführungszeichen drumherum und ohne Erklärungen."
    )
}

// ── Prompts ──────────────────────────────────────────────────────────────────

pub fn build_improve_prompt(settings: &TextImprovementSettings) -> String {
    let mut prompt = if !settings.system_prompt.is_empty() {
        settings.system_prompt.clone()
    } else {
        let tone = match settings.tone.as_str() {
            "formal" => "Verwende einen formellen, professionellen Ton.",
            "casual" => "Verwende einen lockeren, natürlichen Ton.",
            _ => "Verwende einen neutralen, klaren Ton.",
        };
        format!(
            "Du bist ein Lektor und Schreibassistent. Verbessere den folgenden Text:\n\
- Korrigiere Rechtschreibung und Grammatik\n\
- Verbessere die Formulierung und den Lesefluss\n\
- Behalte die ursprüngliche Bedeutung bei\n\
- {tone}\n\
Gib NUR den verbesserten Text zurück, keine Erklärungen."
        )
    };
    if !settings.custom_terms.is_empty() {
        prompt.push_str(&format!(
            "\n\nWichtig: Diese Begriffe exakt so schreiben: {}",
            settings.custom_terms.join(", ")
        ));
    }
    if !settings.context.is_empty() {
        prompt.push_str(&format!("\n\nKontext: {}", settings.context));
    }
    prompt
}

pub fn build_emoji_prompt(density: &str) -> String {
    let d = match density {
        "wenig" => "Setze nur vereinzelt Emojis ein, maximal 1–2 pro Absatz.",
        "viel" => "Setze großzügig Emojis ein, gerne mehrere pro Satz.",
        _ => "Setze regelmäßig passende Emojis ein, etwa alle 1–2 Sätze.",
    };
    format!(
        "Du erhältst ein gesprochenes Transkript. Gib den Text möglichst originalgetreu zurück, \
aber füge passende Emojis ein. {d} \
Korrigiere offensichtliche Fehler. Gib NUR den Text mit Emojis zurück."
    )
}

// ── Weiterleitung ────────────────────────────────────────────────────────────

fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

async fn complete(
    text: &str,
    system: &str,
    cfg: &WorkflowModelConfig,
    temperature: f64,
    app: &AppSettings,
) -> Result<String> {
    match cfg.llm_provider.as_str() {
        llm_provider::GROQ => {
            let key = get_api_key(KEY_GROQ).ok_or_else(|| {
                err("Groq API Key fehlt. Bitte in den Einstellungen hinterlegen.\nKostenlos registrieren: https://console.groq.com")
            })?;
            let model = non_empty(&cfg.llm_model).unwrap_or("llama-3.1-8b-instant");
            openai_compatible(GROQ_URL, Some(&key), model, text, system, temperature).await
        }
        llm_provider::OPENAI => {
            let key = get_api_key(KEY_OPENAI)
                .ok_or_else(|| err("OpenAI API Key fehlt. Bitte in den Einstellungen hinterlegen."))?;
            let model = non_empty(&cfg.llm_model).unwrap_or("gpt-4o-mini");
            openai_compatible(OPENAI_URL, Some(&key), model, text, system, temperature).await
        }
        llm_provider::ANTHROPIC => {
            let key = get_api_key(KEY_ANTHROPIC).ok_or_else(|| {
                err("Anthropic API Key fehlt. Bitte in den Einstellungen hinterlegen.\nhttps://console.anthropic.com")
            })?;
            let model = non_empty(&cfg.llm_model)
                .or(non_empty(&app.anthropic_model))
                .unwrap_or("claude-sonnet-4-5");
            anthropic(ANTHROPIC_URL, &key, model, text, system, temperature).await
        }
        llm_provider::OLLAMA => {
            let base = non_empty(&app.ollama_base_url).unwrap_or("http://localhost:11434");
            let base = base.trim_end_matches('/');
            let model = non_empty(&cfg.llm_model)
                .or(non_empty(&app.ollama_llm_model))
                .unwrap_or("llama3.2");
            let url = format!("{base}/v1/chat/completions");
            openai_compatible(&url, None, model, text, system, temperature).await
        }
        llm_provider::OPENROUTER => {
            let key = get_api_key(KEY_OPENROUTER)
                .ok_or_else(|| err("OpenRouter API Key fehlt. Bitte in den Einstellungen hinterlegen."))?;
            // Ein einziges OpenRouter-Modell, gewählt im Block „OpenRouter – Modell“.
            let model = app.openrouter_llm_model.trim();
            if model.is_empty() {
                return Err(err(
                    "Kein OpenRouter-Modell gewählt. Bitte im Tab API-Zugänge ein Modell auswählen.",
                ));
            }
            openai_compatible(OPENROUTER_CHAT_URL, Some(&key), model, text, system, temperature).await
        }
        llm_provider::CUSTOM => {
            if cfg.custom_llm_url.is_empty() {
                return Err(err("Kein eigener LLM-Endpunkt konfiguriert."));
            }
            let key = get_api_key(KEY_CUSTOM_LLM);
            let model = non_empty(&cfg.llm_model).unwrap_or("gpt-4o-mini");
            openai_compatible(&cfg.custom_llm_url, key.as_deref(), model, text, system, temperature).await
        }
        other => Err(err(format!("Unbekannter LLM-Anbieter: {other}"))),
    }
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

/// Manche Modelle (z. B. OpenAI GPT-5-Reihe, Claude Opus 4.7) lehnen einen
/// eigenen temperature-Wert mit HTTP 400 ab -> dann ohne erneut senden.
fn rejects_temperature(status: u16, body: &str) -> bool {
    status == 400 && body.to_lowercase().contains("temperature")
}

async fn post_json(
    http: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
    payload: &Value,
) -> Result<(u16, String)> {
    let mut req = http.post(url).body(payload.to_string());
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.map_err(|e| err(request_error(e)))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| err(request_error(e)))?;
    Ok((status, body))
}

/// Sendet, und bei abgelehnter temperature noch einmal ohne.
async fn post_with_temperature_fallback(
    url: &str,
    headers: &[(&str, &str)],
    mut payload: Value,
) -> Result<(u16, String)> {
    let http = client(45).map_err(err)?;
    let (status, body) = post_json(&http, url, headers, &payload).await?;
    if rejects_temperature(status, &body) {
        payload.as_object_mut().map(|o| o.shift_remove("temperature"));
        return post_json(&http, url, headers, &payload).await;
    }
    Ok((status, body))
}

async fn openai_compatible(
    url: &str,
    api_key: Option<&str>,
    model: &str,
    text: &str,
    system: &str,
    temperature: f64,
) -> Result<String> {
    let auth = api_key.map(|k| format!("Bearer {k}"));
    let mut headers = vec![("Content-Type", "application/json")];
    if let Some(a) = auth.as_deref() {
        headers.push(("Authorization", a));
    }
    let payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": text},
        ],
        "temperature": temperature,
    });
    let (status, body) = post_with_temperature_fallback(url, &headers, payload).await?;
    if status != 200 {
        return Err(err(format!("Fehler {status}: {}", error_message(&body))));
    }
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["choices"][0]["message"]["content"].as_str().map(|t| t.trim().to_string()))
        .ok_or_else(|| err("Keine Antwort vom LLM erhalten."))
}

async fn anthropic(
    url: &str,
    api_key: &str,
    model: &str,
    text: &str,
    system: &str,
    temperature: f64,
) -> Result<String> {
    let headers = [
        ("x-api-key", api_key),
        ("anthropic-version", "2023-06-01"),
        ("content-type", "application/json"),
    ];
    let payload = json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": [{"role": "user", "content": text}],
        "temperature": temperature,
    });
    let (status, body) = post_with_temperature_fallback(url, &headers, payload).await?;
    if status != 200 {
        return Err(err(format!("Anthropic Fehler {status}: {}", error_message(&body))));
    }
    // Alle Textabschnitte zusammensetzen (1.0.8 las nur den ersten Abschnitt;
    // neuere Modelle können davor andere Abschnitte liefern).
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        let kinds: Vec<&str> = v["content"]
            .as_array()
            .map(|b| b.iter().filter_map(|x| x["type"].as_str()).collect())
            .unwrap_or_default();
        log::warn!(
            "Claude ohne Text: stop_reason={}, Abschnitte={kinds:?}, Modell {model}",
            v["stop_reason"].as_str().unwrap_or("?")
        );
        return Err(err("Keine Antwort von Claude erhalten."));
    }
    Ok(text.trim().to_string())
}

// ── Modelllisten von OpenRouter ──────────────────────────────────────────────

/// Alle Modell-IDs von OpenRouter (sortiert), bei Fehlern leer –
/// wie `fetch_openrouter_models()` in settings.py.
pub async fn fetch_openrouter_models() -> Vec<String> {
    fetch_model_ids(OPENROUTER_MODELS_URL, &[], get_api_key(KEY_OPENROUTER).as_deref()).await
}

/// Holt `data[].id` von einem Modell-Endpunkt; jeder Fehler ergibt eine leere
/// Liste, damit die Oberfläche ohne Vorschläge weiter funktioniert.
pub(crate) async fn fetch_model_ids(url: &str, query: &[(&str, &str)], api_key: Option<&str>) -> Vec<String> {
    let Ok(http) = client(10) else { return Vec::new() };
    let mut req = http.get(url).query(query);
    if let Some(k) = api_key {
        req = req.bearer_auth(k);
    }
    let Ok(resp) = req.send().await else { return Vec::new() };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(v) = resp.json::<Value>().await else { return Vec::new() };
    let mut ids: Vec<String> = v["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m["id"].as_str().filter(|s| !s.is_empty()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server;
    use std::path::Path;

    /// Prompts wörtlich wie in llm.py (Testdaten aus der Python-Version).
    #[test]
    fn prompts_wie_python() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/llm_prompts.json");
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(v["PUNCTUATION_HINT"].as_str().unwrap(), punctuation_hint());
        assert_eq!(v["PUNCTUATION_PROMPT"].as_str().unwrap(), punctuation_prompt());
        for case in v["emoji"].as_array().unwrap() {
            assert_eq!(build_emoji_prompt(case[0].as_str().unwrap()), case[1].as_str().unwrap());
        }
        for case in v["improve"].as_array().unwrap() {
            let s: TextImprovementSettings = TextImprovementSettings {
                system_prompt: case[0]["system_prompt"].as_str().unwrap().into(),
                context: case[0]["context"].as_str().unwrap().into(),
                tone: case[0]["tone"].as_str().unwrap().into(),
                custom_terms: case[0]["custom_terms"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().into())
                    .collect(),
                model: Default::default(),
            };
            assert_eq!(build_improve_prompt(&s), case[1].as_str().unwrap());
        }
        // Voreinstellung = Text aus 1.0.8 + Zusatz (Entscheidung vom 24.11.2026)
        assert_eq!(
            crate::settings::DAMPF_ABLASSEN_PROMPT,
            format!("{} {}", v["DAMPF_DEFAULT"].as_str().unwrap(), crate::settings::DAMPF_KURZ_ZUSATZ)
        );
    }

    #[tokio::test]
    async fn openai_kompatibel_mit_temperature_wiederholung() {
        let srv = test_server::start(vec![
            (400, r#"{"error":{"message":"Unsupported value: 'temperature'"}}"#),
            (200, r#"{"choices":[{"message":{"content":" Antwort "}}]}"#),
        ])
        .await;
        let t = openai_compatible(&srv.url, Some("K"), "gpt-x", "Text", "System", 0.3).await.unwrap();
        assert_eq!(t, "Antwort");
        let reqs = srv.requests.lock().unwrap().clone();
        assert_eq!(reqs[0].header("authorization"), Some("Bearer K"));
        assert_eq!(
            reqs[0].json(),
            json!({"model":"gpt-x","messages":[{"role":"system","content":"System"},{"role":"user","content":"Text"}],"temperature":0.3})
        );
        assert!(reqs[1].json().get("temperature").is_none());
    }

    #[tokio::test]
    async fn openai_kompatibel_fehler_und_ohne_key() {
        let srv = test_server::start(vec![(500, "Serverfehler")]).await;
        let e = openai_compatible(&srv.url, None, "m", "t", "s", 0.0).await.unwrap_err();
        assert_eq!(e.0, "Fehler 500: Serverfehler");
        assert_eq!(srv.requests.lock().unwrap()[0].header("authorization"), None);
    }

    #[tokio::test]
    async fn anthropic_anfrage() {
        let srv = test_server::start(vec![(200, r#"{"content":[{"type":"text","text":" Hallo "}]}"#)]).await;
        let t = anthropic(&srv.url, "AK", "claude-x", "Text", "Sys", 0.4).await.unwrap();
        assert_eq!(t, "Hallo");
        let req = srv.requests.lock().unwrap()[0].clone();
        assert_eq!(req.header("x-api-key"), Some("AK"));
        assert_eq!(req.header("anthropic-version"), Some("2023-06-01"));
        assert_eq!(
            req.json(),
            json!({"model":"claude-x","max_tokens":1024,"system":"Sys","messages":[{"role":"user","content":"Text"}],"temperature":0.4})
        );
    }

    #[tokio::test]
    async fn modellliste_sortiert_und_fehlertolerant() {
        let srv = test_server::start(vec![(200, r#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"},{"x":1}]}"#)]).await;
        let ids = fetch_model_ids(&srv.url, &[("output_modalities", "transcription")], Some("K")).await;
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(srv.requests.lock().unwrap()[0].path, "/?output_modalities=transcription");
        let srv = test_server::start(vec![(500, "x")]).await;
        assert!(fetch_model_ids(&srv.url, &[], None).await.is_empty());
    }

    /// Echter Aufruf bei Groq mit dem gespeicherten Key (kostenloser Dienst).
    #[tokio::test]
    #[ignore = "ruft Groq mit dem echten Key auf; manuell mit --ignored starten"]
    async fn groq_live() {
        let app = AppSettings::default();
        let t = convert_punctuation("hallo komma wie geht es dir fragezeichen", &app.transcription_model, &app)
            .await
            .unwrap();
        println!("Groq-LLM: {t}");
        assert!(!t.is_empty());
    }
}
