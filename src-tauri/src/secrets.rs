//! API-Keys im System-Schlüsselbund – kompatibel zur Python-Version 1.0.8.
//!
//! Die Python-Version nutzt die Bibliothek `keyring` (25.7.0) mit dem
//! Dienstnamen `BlitzVox`. Damit vorhandene Keys nach dem Update gefunden
//! werden, bildet dieses Modul deren Speicherschema exakt nach:
//!
//! **Windows (Anmeldeinformationsverwaltung, `WinVaultKeyring`):**
//! Ein Key liegt unter dem Ziel `BlitzVox`, solange dort kein anderer Key
//! steht. Beim Speichern wird ein vorhandener Eintrag unter `BlitzVox` nach
//! `{benutzer}@BlitzVox` verschoben und der neue unter `BlitzVox` abgelegt.
//! Gelesen wird zuerst `BlitzVox` (nur bei passendem Benutzernamen), sonst
//! `{name}@BlitzVox`. Der Wert ist UTF-16LE ohne Nullzeichen, Typ GENERIC,
//! Persistenz ENTERPRISE.
//!
//! **Linux (Secret Service, z. B. GNOME-Schlüsselbund):** Eintrag in der
//! Standard-Sammlung mit den Attributen `service`, `username` und
//! `application = "Python keyring library"`, Bezeichnung
//! `Password for '{name}' on 'BlitzVox'`, Inhalt UTF-8, Typ `text/plain`.
//! Das Attribut `application` muss gleich bleiben, sonst würde beim
//! Speichern ein zweiter Eintrag statt eines Ersatzes entstehen.

// Wird erst mit den Workflows und der Oberfläche vollständig benutzt.
#![allow(dead_code)]

pub const SERVICE_NAME: &str = "BlitzVox";

pub const KEY_OPENAI: &str = "openai_api_key";
pub const KEY_GROQ: &str = "groq_api_key";
pub const KEY_ANTHROPIC: &str = "anthropic_api_key";
pub const KEY_ASSEMBLYAI: &str = "assemblyai_api_key";
pub const KEY_CUSTOM_STT: &str = "custom_stt_api_key";
pub const KEY_CUSTOM_LLM: &str = "custom_llm_api_key";
pub const KEY_OPENROUTER: &str = "openrouter_api_key";

/// Liest einen Key. Wie `get_api_key()`: bei Fehlern oder leerem Wert `None`.
pub fn get_api_key(name: &str) -> Option<String> {
    match backend::get_password(SERVICE_NAME, name) {
        Ok(Some(v)) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Speichert einen Key. Fehler werden wie in `set_api_key()` weitergegeben.
pub fn set_api_key(name: &str, value: &str) -> Result<(), String> {
    backend::set_password(SERVICE_NAME, name, value)
}

/// Löscht einen Key. Fehler werden wie in `delete_api_key()` ignoriert.
pub fn delete_api_key(name: &str) {
    let _ = backend::delete_password(SERVICE_NAME, name);
}

// ── Windows ──────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod backend {
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_ENTERPRISE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    struct Credential {
        user_name: String,
        blob: Vec<u8>,
    }

    impl Credential {
        /// Wie `DecodingCredential.value`: erst UTF-16, dann UTF-8.
        fn value(&self) -> Result<String, String> {
            if self.blob.len() % 2 == 0 {
                let units: Vec<u16> =
                    self.blob.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                if let Ok(s) = String::from_utf16(&units) {
                    return Ok(s);
                }
            }
            String::from_utf8(self.blob.clone()).map_err(|e| format!("Key nicht lesbar: {e}"))
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut len = 0;
        // SAFETY: Windows liefert nullterminierte Zeichenketten.
        unsafe {
            while *p.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    }

    fn compound_name(username: &str, service: &str) -> String {
        format!("{username}@{service}")
    }

    fn read_credential(target: &str) -> Result<Option<Credential>, String> {
        let target_w = wide(target);
        let mut pcred: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: gültige, nullterminierte Zeichenkette und Ausgabezeiger.
        let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pcred) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!("CredRead fehlgeschlagen (Fehler {err})"));
        }
        // SAFETY: CredReadW war erfolgreich, pcred zeigt auf eine gültige Struktur,
        // die nach dem Kopieren mit CredFree freigegeben wird.
        let cred = unsafe {
            let c = &*pcred;
            let blob = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
            };
            let user_name = from_wide(c.UserName);
            CredFree(pcred as *const _);
            Credential { user_name, blob }
        };
        Ok(Some(cred))
    }

    fn write_credential(target: &str, username: &str, password: &str) -> Result<(), String> {
        let mut target_w = wide(target);
        let mut user_w = wide(username);
        let mut comment_w = wide("Stored using python-keyring");
        // UTF-16LE ohne abschließendes Nullzeichen, wie pywin32-ctypes
        let mut blob: Vec<u8> = password.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        // SAFETY: CREDENTIALW besteht nur aus Zahlen und Zeigern; Nullen sind gültig.
        let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = target_w.as_mut_ptr();
        cred.UserName = user_w.as_mut_ptr();
        cred.Comment = comment_w.as_mut_ptr();
        cred.CredentialBlobSize = blob.len() as u32;
        cred.CredentialBlob = if blob.is_empty() { ptr::null_mut() } else { blob.as_mut_ptr() };
        cred.Persist = CRED_PERSIST_ENTERPRISE;
        // SAFETY: alle Zeiger zeigen auf Puffer, die bis nach dem Aufruf leben.
        let ok = unsafe { CredWriteW(&cred, 0) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return Err(format!("CredWrite fehlgeschlagen (Fehler {err})"));
        }
        Ok(())
    }

    fn delete_credential(target: &str) -> Result<(), String> {
        let target_w = wide(target);
        // SAFETY: gültige, nullterminierte Zeichenkette.
        let ok = unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_NOT_FOUND {
                return Err(format!("CredDelete fehlgeschlagen (Fehler {err})"));
            }
        }
        Ok(())
    }

    fn resolve_credential(service: &str, username: &str) -> Result<Option<Credential>, String> {
        // zuerst unter dem Dienstnamen, sonst unter dem zusammengesetzten Namen
        match read_credential(service)? {
            Some(c) if username.is_empty() || c.user_name == username => Ok(Some(c)),
            _ => read_credential(&compound_name(username, service)),
        }
    }

    pub fn get_password(service: &str, username: &str) -> Result<Option<String>, String> {
        resolve_credential(service, username)?.map(|c| c.value()).transpose()
    }

    pub fn set_password(service: &str, username: &str, password: &str) -> Result<(), String> {
        if let Some(existing) = read_credential(service)? {
            // vorhandenen Eintrag unter dem zusammengesetzten Namen neu speichern
            let target = compound_name(&existing.user_name, service);
            write_credential(&target, &existing.user_name, &existing.value()?)?;
        }
        write_credential(service, username, password)
    }

    pub fn delete_password(service: &str, username: &str) -> Result<(), String> {
        let compound = compound_name(username, service);
        let mut deleted = false;
        for target in [service, compound.as_str()] {
            if let Some(c) = read_credential(target)? {
                if c.user_name == username {
                    deleted = true;
                    delete_credential(target)?;
                }
            }
        }
        if deleted { Ok(()) } else { Err(format!("Kein Eintrag für {service}")) }
    }
}

// ── Linux ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod backend {
    use std::collections::HashMap;

    use secret_service::EncryptionType;
    use secret_service::blocking::{Collection, SecretService};

    const APPID: &str = "Python keyring library";

    fn err(e: impl std::fmt::Display) -> String {
        e.to_string()
    }

    /// Standard-Sammlung, bei Bedarf entsperrt (wie `get_preferred_collection`).
    fn unlocked_collection<'a>(ss: &'a SecretService<'a>) -> Result<Collection<'a>, String> {
        let collection = ss.get_default_collection().map_err(err)?;
        if collection.is_locked().map_err(err)? {
            collection.unlock().map_err(err)?;
            if collection.is_locked().map_err(err)? {
                return Err("Schlüsselbund ist gesperrt (Entsperren abgebrochen).".into());
            }
        }
        Ok(collection)
    }

    fn query<'a>(service: &'a str, username: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([("username", username), ("service", service)])
    }

    pub fn get_password(service: &str, username: &str) -> Result<Option<String>, String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(err)?;
        let collection = unlocked_collection(&ss)?;
        let items = collection.search_items(query(service, username)).map_err(err)?;
        let Some(item) = items.first() else { return Ok(None) };
        item.unlock().map_err(err)?;
        if item.is_locked().map_err(err)? {
            return Err("Eintrag ist gesperrt (Entsperren abgebrochen).".into());
        }
        let secret = item.get_secret().map_err(err)?;
        String::from_utf8(secret).map(Some).map_err(err)
    }

    pub fn set_password(service: &str, username: &str, password: &str) -> Result<(), String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(err)?;
        let collection = unlocked_collection(&ss)?;
        let mut attributes = query(service, username);
        attributes.insert("application", APPID);
        let label = format!("Password for '{username}' on '{service}'");
        collection
            .create_item(&label, attributes, password.as_bytes(), true, "text/plain")
            .map(|_| ())
            .map_err(err)
    }

    pub fn delete_password(service: &str, username: &str) -> Result<(), String> {
        let ss = SecretService::connect(EncryptionType::Dh).map_err(err)?;
        let collection = unlocked_collection(&ss)?;
        let items = collection.search_items(query(service, username)).map_err(err)?;
        match items.first() {
            Some(item) => item.delete().map_err(err), // nur der erste, wie in Python
            None => Err("No such password!".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nur lesend: prüft, dass die von der Python-Version gespeicherten Keys
    /// gefunden und dekodiert werden. Gibt keine Key-Inhalte aus.
    #[test]
    #[ignore = "liest den echten Schlüsselbund; manuell mit --ignored starten"]
    fn vorhandene_keys_lesen() {
        for name in [
            KEY_OPENAI,
            KEY_GROQ,
            KEY_ANTHROPIC,
            KEY_ASSEMBLYAI,
            KEY_CUSTOM_STT,
            KEY_CUSTOM_LLM,
            KEY_OPENROUTER,
        ] {
            let status = match get_api_key(name) {
                Some(k) => format!("gefunden, {} Zeichen", k.chars().count()),
                None => "nicht vorhanden".into(),
            };
            println!("{name}: {status}");
        }
    }

    /// Kreuztest mit Python: erwartet die von Python geschriebenen Einträge
    /// a/b unter `BlitzVoxTest-Interop`, liest sie und schreibt c dazu,
    /// das Python danach wieder lesen muss.
    #[test]
    #[ignore = "nur zusammen mit dem Python-Gegenstück ausführen"]
    fn kreuztest_mit_python() {
        let svc = "BlitzVoxTest-Interop";
        assert_eq!(backend::get_password(svc, "a").unwrap().as_deref(), Some("py-wert-ä"));
        assert_eq!(backend::get_password(svc, "b").unwrap().as_deref(), Some("py-wert-b"));
        backend::set_password(svc, "c", "rust-wert-ö").unwrap();
    }

    /// Schreibt, liest und löscht Test-Einträge unter einem eigenen
    /// Dienstnamen – berührt die echten BlitzVox-Keys nicht.
    #[test]
    #[ignore = "schreibt in den echten Schlüsselbund; manuell mit --ignored starten"]
    fn schema_wie_python() {
        let svc = "BlitzVoxTest-Rust";
        backend::set_password(svc, "eins", "wert-ä-1").unwrap();
        backend::set_password(svc, "zwei", "wert-2").unwrap();
        assert_eq!(backend::get_password(svc, "eins").unwrap().as_deref(), Some("wert-ä-1"));
        assert_eq!(backend::get_password(svc, "zwei").unwrap().as_deref(), Some("wert-2"));
        backend::delete_password(svc, "eins").unwrap();
        backend::delete_password(svc, "zwei").unwrap();
        assert_eq!(backend::get_password(svc, "eins").unwrap(), None);
    }
}
