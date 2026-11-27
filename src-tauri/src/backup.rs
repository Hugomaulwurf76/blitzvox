//! Verschlüsselte Sicherung und Wiederherstellung aller Einstellungen.
//!
//! Gesichert werden `settings.json`, `corrections.json` (gelernte Korrekturen)
//! und alle API-Keys aus dem Schlüsselbund. Die lokalen Whisper-Modelle
//! gehören nicht dazu, sie lassen sich neu herunterladen.
//!
//! **Verschlüsselung:** Aus dem Passwort wird mit Argon2id (Version 1.3,
//! 256 MiB Speicher, 3 Durchläufe, 4 Spuren) und einem zufälligen 32-Byte-Salt
//! ein 256-Bit-Schlüssel abgeleitet. Damit werden die Daten mit AES-256-GCM
//! und einer zufälligen 12-Byte-Nonce verschlüsselt. Salt und Nonce kommen bei
//! jeder Sicherung neu aus dem Zufallsgenerator des Betriebssystems. Der
//! komplette Dateikopf geht als „associated data“ in die Prüfsumme ein: Wird
//! auch nur ein Byte am Kopf oder an den Daten verändert, schlägt das
//! Entschlüsseln fehl.
//!
//! **Dateiformat** (`.bvbak`, alle Zahlen Little Endian):
//!
//! | Bytes | Inhalt                                       |
//! |-------|----------------------------------------------|
//! | 12    | Kennung `BLITZVOX-BAK`                       |
//! | 2     | Formatversion (1)                            |
//! | 1     | Schlüsselableitung (1 = Argon2id v1.3)       |
//! | 4     | Argon2 Speicher in KiB                       |
//! | 4     | Argon2 Durchläufe                            |
//! | 4     | Argon2 Spuren                                |
//! | 32    | Salt                                         |
//! | 1     | Verschlüsselung (1 = AES-256-GCM)            |
//! | 12    | Nonce                                        |
//! | Rest  | verschlüsseltes JSON mit 16 Byte Prüfsumme   |
//!
//! Die Argon2-Parameter stehen im Kopf, damit spätere Versionen stärkere
//! Werte nehmen und alte Sicherungen trotzdem lesen können.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use crate::app::VERSION;
use crate::corrections;
use crate::secrets::{ALL_KEYS, get_api_key, set_api_key};
use crate::settings::{self, AppSettings};

/// Dateiendung der Sicherung.
pub const EXTENSION: &str = "bvbak";
/// Mindestlänge des Passworts in Zeichen.
pub const MIN_PASSWORD_CHARS: usize = 12;

const MAGIC: &[u8; 12] = b"BLITZVOX-BAK";
/// Formatversion des Dateikopfs.
const FILE_VERSION: u16 = 1;
/// Version des JSON-Inhalts.
const CONTENT_VERSION: u32 = 1;
const CONTENT_FORMAT: &str = "blitzvox-backup";
const KDF_ARGON2ID: u8 = 1;
const CIPHER_AES256GCM: u8 = 1;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const HEADER_LEN: usize = 12 + 2 + 1 + 4 + 4 + 4 + SALT_LEN + 1 + NONCE_LEN;
const TAG_LEN: usize = 16;

/// Obergrenzen beim Lesen: Eine manipulierte Datei soll das Programm nicht
/// mit absurden Werten (z. B. 4 TiB Speicher) lahmlegen können.
const MAX_M_COST_KIB: u32 = 4 * 1024 * 1024; // 4 GiB
const MAX_T_COST: u32 = 64;
const MAX_P_COST: u32 = 64;

/// Parameter der Schlüsselableitung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

/// Standard für neue Sicherungen: 256 MiB, 3 Durchläufe, 4 Spuren.
pub const DEFAULT_KDF: KdfParams = KdfParams { m_cost_kib: 256 * 1024, t_cost: 3, p_cost: 4 };

// ── Fehler ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum BackupError {
    /// Datei ist keine BlitzVox-Sicherung.
    NotABackup,
    /// Datei stammt von einer neueren BlitzVox-Version.
    NewerFormat,
    /// Unbekanntes Verfahren oder unzulässige Parameter im Kopf.
    Unsupported(String),
    /// Prüfsumme stimmt nicht: falsches Passwort oder veränderte Datei.
    WrongPasswordOrDamaged,
    /// Entschlüsselt, aber der Inhalt ist ungültig.
    InvalidContent(String),
    /// Passwort zu kurz (nur beim Erstellen).
    PasswordTooShort,
    /// Lesen, Schreiben oder Zufallsgenerator fehlgeschlagen.
    Io(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotABackup => write!(f, "Die Datei ist keine BlitzVox-Sicherung."),
            Self::NewerFormat => write!(
                f,
                "Die Sicherung stammt von einer neueren BlitzVox-Version. Bitte BlitzVox zuerst aktualisieren."
            ),
            Self::Unsupported(e) => write!(f, "Die Sicherung kann nicht gelesen werden: {e}"),
            Self::WrongPasswordOrDamaged => write!(
                f,
                "Passwort falsch oder Datei beschädigt. Es wurde nichts geändert."
            ),
            Self::InvalidContent(e) => write!(f, "Der Inhalt der Sicherung ist ungültig: {e}"),
            Self::PasswordTooShort => {
                write!(f, "Das Passwort muss mindestens {MIN_PASSWORD_CHARS} Zeichen lang sein.")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<BackupError> for String {
    fn from(e: BackupError) -> Self {
        e.to_string()
    }
}

// ── Inhalt ───────────────────────────────────────────────────────────────────

/// Klartext der Sicherung. Die Keys werden beim Verwerfen überschrieben.
#[derive(Serialize, Deserialize)]
pub struct BackupContent {
    pub format: String,
    pub version: u32,
    /// Zeitpunkt der Sicherung in Sekunden seit 1970 (UTC).
    pub created: u64,
    pub app_version: String,
    pub settings: Value,
    pub corrections: Value,
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

impl Drop for BackupContent {
    fn drop(&mut self) {
        for v in self.keys.values_mut() {
            v.zeroize();
        }
    }
}

impl BackupContent {
    /// Liest den gespeicherten Stand: Dateien von der Platte, Keys aus dem
    /// Schlüsselbund (nur gesetzte Keys).
    pub fn collect() -> Result<Self, BackupError> {
        let settings = settings::load_settings();
        let corrections = corrections::load();
        let keys = ALL_KEYS
            .iter()
            .filter_map(|k| get_api_key(k).map(|v| (k.to_string(), v)))
            .collect();
        Ok(Self {
            format: CONTENT_FORMAT.into(),
            version: CONTENT_VERSION,
            created: now_secs(),
            app_version: VERSION.into(),
            settings: serde_json::to_value(&settings).map_err(|e| BackupError::Io(e.to_string()))?,
            corrections: serde_json::to_value(&corrections)
                .map_err(|e| BackupError::Io(e.to_string()))?,
            keys,
        })
    }

    fn to_json(&self) -> Result<Zeroizing<Vec<u8>>, BackupError> {
        serde_json::to_vec(self).map(Zeroizing::new).map_err(|e| BackupError::Io(e.to_string()))
    }

    fn from_json(bytes: &[u8]) -> Result<Self, BackupError> {
        let c: Self =
            serde_json::from_slice(bytes).map_err(|e| BackupError::InvalidContent(e.to_string()))?;
        if c.format != CONTENT_FORMAT {
            return Err(BackupError::InvalidContent("unbekanntes Format".into()));
        }
        if c.version > CONTENT_VERSION {
            return Err(BackupError::NewerFormat);
        }
        Ok(c)
    }

    /// Einstellungen der Sicherung, wie beim Laden mit den Standardwerten
    /// ergänzt (fehlende Felder) und ohne unbekannte Felder.
    pub fn parsed_settings(&self) -> Result<AppSettings, BackupError> {
        settings::parse_merged(&self.settings.to_string()).map_err(BackupError::InvalidContent)
    }

    /// Anzahl der Keys, die beim Einspielen geschrieben werden.
    pub fn known_key_count(&self) -> usize {
        self.keys.iter().filter(|(k, v)| ALL_KEYS.contains(&k.as_str()) && !v.is_empty()).count()
    }

    pub fn correction_count(&self) -> usize {
        corrections::from_value(self.corrections.clone()).replacements.len()
    }

    /// Kurzbeschreibung für die Rückfrage vor dem Einspielen.
    pub fn summary(&self) -> String {
        let (y, m, d) = utc_date(self.created);
        format!(
            "Sicherung vom {d:02}.{m:02}.{y} (BlitzVox {})\n\
             – Einstellungen\n– {} gelernte Korrekturen\n– {} API-Keys",
            self.app_version,
            self.correction_count(),
            self.known_key_count()
        )
    }
}

// ── Verschlüsselung ──────────────────────────────────────────────────────────

fn random_bytes<const N: usize>() -> Result<[u8; N], BackupError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| BackupError::Io(format!("Zufallsgenerator: {e}")))?;
    Ok(b)
}

fn derive_key(
    password: &str,
    salt: &[u8],
    p: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, BackupError> {
    let params = Params::new(p.m_cost_kib, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| BackupError::Unsupported(e.to_string()))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| BackupError::Unsupported(e.to_string()))?;
    Ok(key)
}

fn header(p: KdfParams, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER_LEN);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(&FILE_VERSION.to_le_bytes());
    h.push(KDF_ARGON2ID);
    h.extend_from_slice(&p.m_cost_kib.to_le_bytes());
    h.extend_from_slice(&p.t_cost.to_le_bytes());
    h.extend_from_slice(&p.p_cost.to_le_bytes());
    h.extend_from_slice(salt);
    h.push(CIPHER_AES256GCM);
    h.extend_from_slice(nonce);
    debug_assert_eq!(h.len(), HEADER_LEN);
    h
}

/// Verschlüsselt `plain` mit dem Passwort. Liefert die komplette Datei.
pub fn encrypt(plain: &[u8], password: &str, p: KdfParams) -> Result<Vec<u8>, BackupError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(BackupError::PasswordTooShort);
    }
    let salt = random_bytes::<SALT_LEN>()?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    let key = derive_key(password, &salt, p)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| BackupError::Io(e.to_string()))?;
    let mut out = header(p, &salt, &nonce);
    let ct = cipher
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: &out })
        .map_err(|_| BackupError::Io("Verschlüsseln fehlgeschlagen".into()))?;
    out.extend_from_slice(&ct);
    Ok(out)
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Entschlüsselt eine komplette Datei.
pub fn decrypt(data: &[u8], password: &str) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err(BackupError::NotABackup);
    }
    if data.len() < 14 {
        return Err(BackupError::WrongPasswordOrDamaged);
    }
    let version = u16::from_le_bytes([data[12], data[13]]);
    if version > FILE_VERSION {
        return Err(BackupError::NewerFormat);
    }
    if version == 0 {
        return Err(BackupError::Unsupported("Formatversion 0".into()));
    }
    if data.len() < HEADER_LEN + TAG_LEN {
        return Err(BackupError::WrongPasswordOrDamaged);
    }
    let (head, ct) = data.split_at(HEADER_LEN);
    if head[14] != KDF_ARGON2ID {
        return Err(BackupError::Unsupported(format!("Schlüsselableitung {}", head[14])));
    }
    let p = KdfParams { m_cost_kib: read_u32(head, 15), t_cost: read_u32(head, 19), p_cost: read_u32(head, 23) };
    if p.m_cost_kib > MAX_M_COST_KIB || p.t_cost > MAX_T_COST || p.p_cost > MAX_P_COST {
        return Err(BackupError::Unsupported("Argon2-Parameter außerhalb der Grenzen".into()));
    }
    let salt = &head[27..27 + SALT_LEN];
    let cipher_id = head[27 + SALT_LEN];
    if cipher_id != CIPHER_AES256GCM {
        return Err(BackupError::Unsupported(format!("Verschlüsselung {cipher_id}")));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&head[28 + SALT_LEN..]);
    let key = derive_key(password, salt, p)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| BackupError::Io(e.to_string()))?;
    cipher
        .decrypt(&Nonce::from(nonce), Payload { msg: ct, aad: head })
        .map(Zeroizing::new)
        .map_err(|_| BackupError::WrongPasswordOrDamaged)
}

// ── Datei ────────────────────────────────────────────────────────────────────

/// Hängt `.bvbak` an, wenn der Dialog keine Endung geliefert hat.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(EXTENSION);
        PathBuf::from(s)
    }
}

/// Vorschlag für den Dateinamen, z. B. `BlitzVox-Sicherung-2026-11-27.bvbak`.
pub fn default_file_name() -> String {
    let (y, m, d) = utc_date(now_secs());
    format!("BlitzVox-Sicherung-{y}-{m:02}-{d:02}.{EXTENSION}")
}

/// Schreibt `content` verschlüsselt nach `path`. Erst in eine Zwischendatei,
/// dann umbenennen, damit eine vorhandene Sicherung nie halb überschrieben wird.
pub fn write_to(
    path: &Path,
    content: &BackupContent,
    password: &str,
    p: KdfParams,
) -> Result<(), BackupError> {
    let plain = content.to_json()?;
    let data = encrypt(&plain, password, p)?;
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    fs::write(&tmp, &data).map_err(|e| BackupError::Io(format!("Speichern fehlgeschlagen: {e}")))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        BackupError::Io(format!("Speichern fehlgeschlagen: {e}"))
    })
}

/// Erstellt eine Sicherung des gespeicherten Stands.
pub fn create(path: &Path, password: &str) -> Result<(), BackupError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(BackupError::PasswordTooShort);
    }
    write_to(path, &BackupContent::collect()?, password, DEFAULT_KDF)
}

/// Liest und entschlüsselt eine Sicherung. Ändert nichts.
pub fn read_from(path: &Path, password: &str) -> Result<BackupContent, BackupError> {
    let data = fs::read(path).map_err(|e| BackupError::Io(format!("Datei nicht lesbar: {e}")))?;
    let plain = decrypt(&data, password)?;
    let content = BackupContent::from_json(&plain)?;
    content.parsed_settings()?; // ungültige Einstellungen schon hier ablehnen
    Ok(content)
}

// ── Einspielen ───────────────────────────────────────────────────────────────

/// Schreibt Einstellungen, Korrekturen und Keys der Sicherung.
///
/// Vorher werden die aktuellen Dateien als `*.vor-wiederherstellung.json`
/// kopiert. Keys, die nicht in der Sicherung stehen, bleiben erhalten.
/// Autostart und Tastenkürzel richtet der Aufrufer neu ein.
pub fn apply(content: &BackupContent) -> Result<AppSettings, BackupError> {
    let new_settings = content.parsed_settings()?;
    let new_corrections = corrections::from_value(content.corrections.clone());
    let io = |what: &str, e: std::io::Error| BackupError::Io(format!("{what}: {e}"));

    let settings_path = settings::settings_path().map_err(|e| io("Konfigurationsordner", e))?;
    let corrections_path = corrections::corrections_path().map_err(|e| io("Konfigurationsordner", e))?;
    for p in [&settings_path, &corrections_path] {
        if p.exists() {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("datei");
            let copy = p.with_file_name(format!("{stem}.vor-wiederherstellung.json"));
            fs::copy(p, &copy).map_err(|e| io("Sicherheitskopie", e))?;
        }
    }

    for (name, value) in &content.keys {
        if ALL_KEYS.contains(&name.as_str()) && !value.trim().is_empty() {
            set_api_key(name, value.trim()).map_err(BackupError::Io)?;
        }
    }
    corrections::save_to(&corrections_path, &new_corrections).map_err(|e| io("Korrekturen", e))?;
    settings::save_to(&settings_path, &new_settings).map_err(|e| io("Einstellungen", e))?;
    Ok(new_settings)
}

// ── Zeit ─────────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Datum (Jahr, Monat, Tag) in UTC, Algorithmus „civil_from_days“
/// von Howard Hinnant.
fn utc_date(secs: u64) -> (i64, u32, u32) {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kleine Parameter, damit die Tests schnell laufen.
    const FAST: KdfParams = KdfParams { m_cost_kib: 64, t_cost: 1, p_cost: 1 };
    const PW: &str = "richtig-gutes-Passwort";

    fn sample() -> BackupContent {
        BackupContent {
            format: CONTENT_FORMAT.into(),
            version: CONTENT_VERSION,
            created: 1_795_759_200,
            app_version: "2.0.0".into(),
            settings: serde_json::to_value(AppSettings::default()).unwrap(),
            corrections: serde_json::json!({
                "replacements": { "Proxmocks": "Proxmox" },
                "vocabulary": ["Proxmox"]
            }),
            keys: BTreeMap::from([
                ("groq_api_key".to_string(), "gsk_test-ä".to_string()),
                ("unbekannt".to_string(), "x".to_string()),
            ]),
        }
    }

    #[test]
    fn hin_und_zurueck() {
        let plain = sample().to_json().unwrap();
        let data = encrypt(&plain, PW, FAST).unwrap();
        assert_eq!(data.len(), HEADER_LEN + plain.len() + TAG_LEN);
        let back = decrypt(&data, PW).unwrap();
        assert_eq!(&*back, &*plain);
        let c = BackupContent::from_json(&back).unwrap();
        assert_eq!(c.keys["groq_api_key"], "gsk_test-ä");
        assert_eq!(c.known_key_count(), 1);
        assert_eq!(c.correction_count(), 1);
        c.parsed_settings().unwrap();
    }

    #[test]
    fn klartext_steht_nicht_in_der_datei() {
        let plain = sample().to_json().unwrap();
        let data = encrypt(&plain, PW, FAST).unwrap();
        let needle = b"gsk_test";
        assert!(!data.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn salt_und_nonce_jedes_mal_neu() {
        let a = encrypt(b"gleich", PW, FAST).unwrap();
        let b = encrypt(b"gleich", PW, FAST).unwrap();
        assert_ne!(a[27..27 + SALT_LEN], b[27..27 + SALT_LEN], "Salt");
        assert_ne!(a[28 + SALT_LEN..HEADER_LEN], b[28 + SALT_LEN..HEADER_LEN], "Nonce");
        assert_ne!(a[HEADER_LEN..], b[HEADER_LEN..], "Daten");
    }

    #[test]
    fn falsches_passwort() {
        let data = encrypt(b"geheim", PW, FAST).unwrap();
        assert_eq!(decrypt(&data, "falsches-Passwort!").unwrap_err(), BackupError::WrongPasswordOrDamaged);
    }

    #[test]
    fn jedes_veraenderte_byte_wird_erkannt() {
        let data = encrypt(b"geheim", PW, FAST).unwrap();
        // Kennung und Version ausgenommen, die haben eigene Fehler
        for i in 14..data.len() {
            let mut d = data.clone();
            d[i] ^= 0x01;
            assert!(decrypt(&d, PW).is_err(), "Byte {i} nicht erkannt");
        }
    }

    #[test]
    fn abgeschnittene_datei() {
        let data = encrypt(b"geheim", PW, FAST).unwrap();
        for len in [0, 5, 13, HEADER_LEN, data.len() - 1] {
            assert!(decrypt(&data[..len], PW).is_err(), "Länge {len}");
        }
    }

    #[test]
    fn keine_sicherung() {
        assert_eq!(decrypt(b"{\"irgendwas\": 1}", PW).unwrap_err(), BackupError::NotABackup);
    }

    #[test]
    fn neueres_format_wird_abgelehnt() {
        let mut data = encrypt(b"geheim", PW, FAST).unwrap();
        data[12..14].copy_from_slice(&(FILE_VERSION + 1).to_le_bytes());
        assert_eq!(decrypt(&data, PW).unwrap_err(), BackupError::NewerFormat);

        let mut c = sample();
        c.version = CONTENT_VERSION + 1;
        let json = c.to_json().unwrap();
        assert!(matches!(BackupContent::from_json(&json), Err(BackupError::NewerFormat)));
    }

    #[test]
    fn absurde_parameter_werden_abgelehnt() {
        let mut data = encrypt(b"geheim", PW, FAST).unwrap();
        data[15..19].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decrypt(&data, PW), Err(BackupError::Unsupported(_))));
    }

    #[test]
    fn passwort_zu_kurz() {
        assert_eq!(encrypt(b"x", "elf-Zeichen", FAST).unwrap_err(), BackupError::PasswordTooShort);
        // 12 Zeichen, auch mit Umlauten (Zeichen, nicht Bytes)
        assert!(encrypt(b"x", "zwölf-Zeichä", FAST).is_ok());
    }

    #[test]
    fn datei_schreiben_und_lesen() {
        let d = std::env::temp_dir().join(format!("blitzvox-backup-test-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let p = d.join("test.bvbak");
        write_to(&p, &sample(), PW, FAST).unwrap();
        assert!(!d.join("test.bvbak.tmp").exists());
        let c = read_from(&p, PW).unwrap();
        assert_eq!(c.app_version, "2.0.0");
        assert_eq!(read_from(&p, "falsches-Passwort!").err(), Some(BackupError::WrongPasswordOrDamaged));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn endung_und_dateiname() {
        assert_eq!(with_extension(PathBuf::from("a/sich")), PathBuf::from("a/sich.bvbak"));
        assert_eq!(with_extension(PathBuf::from("a/sich.BVBAK")), PathBuf::from("a/sich.BVBAK"));
        assert_eq!(with_extension(PathBuf::from("a/sich.txt")), PathBuf::from("a/sich.txt.bvbak"));
        assert!(default_file_name().starts_with("BlitzVox-Sicherung-"));
    }

    #[test]
    fn datum() {
        assert_eq!(utc_date(0), (1970, 1, 1));
        assert_eq!(utc_date(951_782_400), (2000, 2, 29));
        assert_eq!(utc_date(1_795_759_200), (2026, 11, 27));
    }

    /// Misst die Dauer mit den echten Parametern (256 MiB). Manuell starten:
    /// `cargo test --release --lib backup::tests::dauer -- --ignored --nocapture`
    #[test]
    #[ignore = "dauert mit den echten Parametern; manuell mit --ignored starten"]
    fn dauer_standardparameter() {
        let start = std::time::Instant::now();
        let data = encrypt(b"messung", PW, DEFAULT_KDF).unwrap();
        let enc = start.elapsed();
        let start = std::time::Instant::now();
        decrypt(&data, PW).unwrap();
        println!("Verschlüsseln: {enc:?}, Entschlüsseln: {:?}", start.elapsed());
    }
}
