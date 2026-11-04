# BlitzVox Cross-Platform

**Speech-to-text desktop app for Windows 11 and Linux Debian.**

A Python/PyQt6 fork of the concept from [cmagnussen/blitztext-app](https://github.com/cmagnussen/blitztext-app) (macOS original, MIT license).

> Bring your own API key — no backend, no telemetry, no warranty.

---

## Was die App macht

| Workflow | Hotkey (Standard) | Beschreibung |
|---|---|---|
| 🎙 **BlitzVox** | `Ctrl+Alt+W` | Sprache → Text (nur Transkription) |
| ✨ **BlitzVox+** | `Ctrl+Alt+B` | Sprache → Transkription → Text verbessert |
| 🔥 **BlitzVox $%&!** | `Ctrl+Alt+F` | Frustrierte Sprache → ruhige Nachricht |
| 😊 **BlitzVox :)** | `Ctrl+Alt+J` | Sprache → Text mit passenden Emojis |
| ✏ **Letzten Text korrigieren** | `Ctrl+Alt+K` | Korrekturfenster öffnen, BlitzVox lernt daraus |

---

## Satzzeichen diktieren

Satzzeichen können gesprochen werden. Ein Sprachmodell entscheidet anhand des Satzbaus, ob ein Wort als Befehl oder als normales Wort gemeint ist („Mir fehlt das Komma“ bleibt stehen).

| Sprich | Ergebnis |
|---|---|
| Komma, Punkt, Fragezeichen, Ausrufezeichen | `,` `.` `?` `!` |
| Doppelpunkt, Semikolon / Strichpunkt | `:` `;` |
| Bindestrich, Gedankenstrich, Auslassungspunkte | `-` `–` `…` |
| neue Zeile, neuer Absatz | Zeilenumbruch, Leerzeile |
| Klammer auf, Klammer zu | `(` `)` |
| Anführungszeichen auf / unten, Anführungszeichen zu / oben | `„` `“` |

Beispiel: „Liebe Grüße Komma neue Zeile Christian“ → „Liebe Grüße,“ + Zeilenumbruch + „Christian“.

Bei **BlitzVox** (reine Transkription) setzt das Sprachmodell aus „BlitzVox+“ bei jedem Diktat die Satzzeichen und die Groß-/Kleinschreibung. Es darf dabei keine Wörter ändern: Ändert es doch eines, wird seine Antwort verworfen und einfache Regeln übernehmen. Dasselbe passiert, wenn das Sprachmodell nicht erreichbar ist. Abschaltbar unter Einstellungen → Korrekturen.

---

## Korrekturen lernen

BlitzVox lernt aus deinen Korrekturen, z. B. wenn ein Name oder Fachbegriff immer falsch erkannt wird.

1. `Ctrl+Alt+K` oder Rechtsklick auf das Tray-Icon → **„Letzten Text korrigieren …“**
2. Text im Fenster korrigieren. BlitzVox zeigt Vorschläge wie „Proxmocks → Proxmox“.
3. Nur echte Hörfehler anhaken (standardmäßig ist nichts angehakt, damit Stiländerungen wie „dann → danach“ nicht gelernt werden).
4. **„Übernehmen & kopieren“** speichert die Haken und legt den korrigierten Text in die Zwischenablage.

Gelernte Korrekturen werden nach jeder Erkennung ersetzt. Die richtige Schreibweise geht außerdem als Hinweis an die Spracherkennung (Groq, OpenAI, eigener Endpunkt und lokal per Prompt, AssemblyAI per `keyterms_prompt`; OpenRouter unterstützt keine Hinweise). Ansehen, hinzufügen und löschen: Einstellungen → Korrekturen.

Reine Groß-/Kleinschreibung und Satzzeichen werden nicht gelernt, das erledigt das Sprachmodell.

---

## Unterstützte Anbieter

### Transkription (STT)
| Anbieter | Was du brauchst |
|---|---|
| **Groq Whisper** *(Standard, kostenlos)* | Groq API Key |
| **OpenAI Whisper API** | OpenAI API Key |
| **AssemblyAI** | AssemblyAI API Key |
| **Lokal (faster-whisper)** | `pip install faster-whisper`, kein Key nötig |
| **Eigener Endpunkt** | OpenAI-kompatibler Endpunkt |

### LLM (Textverarbeitung)
| Anbieter | Was du brauchst |
|---|---|
| **Groq** *(Standard, kostenlos)* | Groq API Key |
| **OpenAI** | OpenAI API Key |
| **Claude (Anthropic)** | Anthropic API Key |
| **Ollama** (lokal, selbst gehostet) | Ollama läuft auf localhost:11434 |
| **Eigener Endpunkt** | OpenAI-kompatibler Endpunkt |

Jeden Workflow kannst du **unabhängig** konfigurieren — z.B. BlitzVox mit Groq-Whisper, BlitzVox+ mit lokalem Ollama.

> ⚠️ Bei Groq: Modellauswahl in den Einstellungen ist eine editierbare Vorschlagsliste. Groq deprecatet Modelle regelmäßig — falls ein Modell mit „404 does not exist" fehlschlägt, ein anderes aus der Liste wählen bzw. auf console.groq.com/docs/models nachsehen, welches aktuell auf deinem Tarif (Free/Developer vs. Enterprise) verfügbar ist.

---

## Installation

### Voraussetzungen
- Für das Setup unter Windows: nichts. Das Setup bringt alles mit.
- Für Entwicklung aus dem Quellcode: Python 3.10 oder neuer
- Auf Linux: das Install-Skript richtet `portaudio19-dev`, `xdotool`/`wtype` etc. automatisch ein (per `sudo apt-get`)

### Windows 11 (fertiges Setup)
1. `BlitzVoxSetup-1.0.7.exe` aus den Releases herunterladen und ausführen.
2. Windows fragt nach Administratorrechten. Die Installation erfolgt immer für alle Benutzer nach `C:\Program Files\BlitzVox`.
3. Der Assistent legt einen Startmenü-Eintrag an, optional auch eine Desktop-Verknüpfung.
4. Autostart mit Windows: Einstellungen → Erweitert → „Mit Windows starten“.

### Windows 11 (aus dem Quellcode)
```cmd
git clone https://github.com/Hugomaulwurf76/blitzvox.git
cd blitzvox
install_windows.bat
start.bat
```

`install_windows.bat` installiert die Python-Pakete in das gefundene Python und erzeugt `start.bat`. Ist kein passendes Python vorhanden, verlinkt das Skript direkt zum Download.

### Linux (Debian/Ubuntu, fertiges Paket)
`blitzvox_1.0.7_amd64.deb` aus den Releases herunterladen und installieren:
```bash
sudo apt install ./blitzvox_1.0.7_amd64.deb
```
Danach im Anwendungsmenü „BlitzVox“ starten. Deinstallation: `sudo apt remove blitzvox`.

### Windows-Setup selbst bauen (Entwickler)
Im Ordner `BlitzVoxEXE`:
1. `build_exe.bat` ausführen. Es installiert die Build-Pakete, erzeugt `BlitzVox.ico` aus dem Tray-Icon und baut `dist\BlitzVox\BlitzVox.exe` mit PyInstaller.
2. `BlitzVox.iss` mit dem [Inno Setup Compiler](https://jrsoftware.org/isdl.php) öffnen und kompilieren. Das Ergebnis liegt in `output\BlitzVoxSetup-1.0.7.exe`.

### Linux (Debian/Ubuntu)
```bash
git clone https://github.com/Hugomaulwurf76/blitzvox.git
cd blitzvox
chmod +x install_linux.sh
./install_linux.sh
./start.sh
```

`install_linux.sh` erkennt automatisch ein passendes Python (≥3.10), legt ein eigenes `.venv` an, installiert alle System- und Python-Abhängigkeiten und erzeugt `start.sh`.

Autostart beim Anmelden: Einstellungen → Erweitert → „Beim Anmelden starten“. Das legt `~/.config/autostart/blitzvox.desktop` an und entfernt die Datei wieder, wenn du die Option ausschaltest.

### Lokale Transkription (kein API Key)
```bash
pip install faster-whisper
```
Dann in den Einstellungen → Workflows → STT-Anbieter: "Lokal (faster-whisper)" wählen.

---

## Erster Start

1. App starten: Startmenü bzw. Desktop-Verknüpfung (Windows), `./start.sh` (Linux)
2. Tray-Icon erscheint in der Taskleiste. Windows 11 legt neue Tray-Icons oft im Überlauf (`^`) ab. Unter „Einstellungen → Personalisierung → Taskleiste → Andere Taskleistensymbole“ lässt es sich dauerhaft einblenden.
3. Rechtsklick → Einstellungen → API-Zugänge → API Key eintragen
4. Rechtsklick → BlitzVox → Aufnahme starten

Unter Linux öffnet ein Linksklick auf das Tray-Icon kein Popup (Wayland-Einschränkung). Dort nutzt du den Rechtsklick.

---

## Konfiguration

Einstellungen werden gespeichert in:
- **Linux:** `~/.config/blitzvox/settings.json`
- **Windows:** `%APPDATA%\blitzvox\settings.json`

Gelernte Korrekturen liegen im selben Ordner in `corrections.json`.

API Keys werden im System-Keyring gespeichert (sicher, nicht im Klartext) — nicht in der `settings.json`.

---

## Projektstruktur

```
main.py                         Einstiegspunkt
install_windows.bat             Quellcode-Setup für Windows (Entwickler)
install_linux.sh                Automatisches Setup für Linux
requirements.txt                Python-Abhängigkeiten (pyautogui/pywin32 nur unter Windows)
BlitzVoxEXE/
  build_exe.bat                 baut dist\BlitzVox\BlitzVox.exe (Entwickler)
  make_icon.py                  erzeugt BlitzVox.ico aus dem Tray-Icon
  BlitzVox.spec                PyInstaller-Konfiguration
  BlitzVox.iss                 Inno-Setup-Skript für BlitzVoxSetup-x.y.z.exe
blitzvox/
  workflow.py                   Kernlogik aller 4 Workflows
  services/
    settings.py                 Konfiguration + Keyring + Modell-Vorschlagslisten
    recorder.py                 Audioaufnahme (sounddevice)
    stt.py                      STT-Anbieter (Groq/OpenAI/AssemblyAI/lokal/custom)
    llm.py                      LLM-Anbieter (Groq/OpenAI/Claude/Ollama/custom)
    corrections.py              Satzzeichen-Befehle, Korrektur-Wörterbuch, Lernlogik
    hotkeys.py                  Globale Hotkeys (pynput)
    paste.py                    Clipboard + Auto-Paste
    autostart.py                Autostart mit Windows (Registry-Eintrag, Windows)
  ui/
    tray.py                     System-Tray + Menü
    workflow_window.py          Aufnahme-UI mit Waveform
    settings_window.py          Einstellungs-UI
    correction_window.py        Fenster „Letzten Text korrigieren“
```

---

## Datenfluss

```
Online (Groq/OpenAI/Claude):  Dein PC → gewählter API-Anbieter
Lokal (Ollama/faster-whisper): Dein PC → läuft komplett auf deiner eigenen Hardware
```

Kein eigenes Backend. Audio und Text gehen direkt vom PC zum gewählten Anbieter.

---

## Lizenz

MIT License — abgeleitet vom Original [cmagnussen/blitztext-app](https://github.com/cmagnussen/blitztext-app).
