//! Gemeinsame HTTP-Hilfen für Spracherkennung und Sprachmodelle.

// Wird erst mit den Workflows vollständig benutzt.
#![allow(dead_code)]

use std::time::Duration;

/// HTTP-Client mit Zeitlimit wie `httpx.AsyncClient(timeout=…)`: das Limit
/// gilt für den Verbindungsaufbau und für jede Wartezeit beim Lesen, nicht
/// für die gesamte Anfrage.
pub fn client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    let t = Duration::from_secs(timeout_secs);
    reqwest::Client::builder()
        .connect_timeout(t)
        .read_timeout(t)
        .build()
        .map_err(|e| e.to_string())
}

/// Text einer Fehlerantwort wie in Python:
/// `resp.json().get("error", {}).get("message") or resp.text[:200]`.
pub fn error_message(body: &str) -> String {
    let from_json = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| {
        v.get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
            .map(String::from)
    });
    from_json.unwrap_or_else(|| body.chars().take(200).collect())
}

/// Fehlertext einer Verbindung (kein HTTP-Status), z. B. Zeitüberschreitung.
pub fn request_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Zeitüberschreitung bei der Anfrage.".into()
    } else if e.is_connect() {
        format!("Keine Verbindung: {e}")
    } else {
        e.to_string()
    }
}

/// Kleiner HTTP-Server für Tests: nimmt Anfragen an, merkt sie sich und
/// antwortet der Reihe nach mit den vorgegebenen Antworten.
#[cfg(test)]
pub mod test_server {
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug, Clone)]
    pub struct Request {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Request {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
        pub fn json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.body).expect("JSON-Körper")
        }
        pub fn body_text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }
    }

    pub struct Server {
        pub url: String,
        pub requests: Arc<Mutex<Vec<Request>>>,
    }

    /// Startet den Server; `responses` sind (Status, Körper).
    pub async fn start(responses: Vec<(u16, &'static str)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let rec = requests.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let req = read_request(&mut sock).await;
                rec.lock().unwrap().push(req);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        Server { url, requests }
    }

    async fn read_request(sock: &mut tokio::net::TcpStream) -> Request {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let head_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
            if n == 0 {
                break buf.len();
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let first = lines.next().unwrap_or("");
        let mut parts = first.split(' ');
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let headers: Vec<(String, String)> = lines
            .filter(|l| !l.is_empty())
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut body = buf[head_end..].to_vec();
        while body.len() < len {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Request { method, path, headers, body }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fehlertext_wie_python() {
        assert_eq!(error_message(r#"{"error":{"message":"Ungültiger Key"}}"#), "Ungültiger Key");
        // AssemblyAI: error ist ein Text -> Python fällt auf den Rohtext zurück
        assert_eq!(error_message(r#"{"error":"kaputt"}"#), r#"{"error":"kaputt"}"#);
        assert_eq!(error_message(&"x".repeat(300)).len(), 200);
    }
}
