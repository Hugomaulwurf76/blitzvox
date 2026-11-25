# BlitzVox

**Spracheingabe per Tastenkürzel für Windows 11 und Linux (Debian, Ubuntu, Linux Mint).**

Inspiriert von [cmagnussen/blitztext-app](https://github.com/cmagnussen/blitztext-app) (macOS-Original, MIT-Lizenz).

> Eigene API-Keys oder komplett lokal – kein eigenes Backend, keine Telemetrie, keine Gewähr.

Ab Version 2.0 ist BlitzVox in Rust mit [Tauri 2](https://v2.tauri.app) neu geschrieben. Funktionen und Bedienung entsprechen 1.0.8, Einstellungen, API-Keys und gelernte Korrekturen werden beim Update übernommen. Die frühere Python-Version liegt unter dem Tag [`v1.0.8`](https://github.com/Hugomaulwurf76/blitzvox/tree/v1.0.8).

---

## Was die App macht

| Workflow | Tastenkürzel (Standard) | Beschreibung |
|---|---|---|
| 🎙 **BlitzVox** | `Strg+Alt+W` | Sprache → Text (nur Transkription) |
| ✨ **BlitzVox+** | `Strg+Alt+B` | Sprache → Transkription → verbesserter Text |
| 🔥 **BlitzVox $%&!** | `Strg+Alt+F` | Frustrierte Sprache → ruhige Nachricht |
| 😊 **BlitzVox :)** | `Strg+Alt+J` | Sprache → Text mit passenden Emojis |
| ✏ **Letzten Text korrigieren** | `Strg+Alt+K` | Korrekturfenster öffnen, BlitzVox lernt daraus |

Jedes Kürzel kann **gehalten** (sprechen, solange die Tasten gedrückt sind) oder **umgeschaltet** werden (einmal drücken zum Starten, noch einmal zum Beenden). Der Text landet in der Zwischenablage und wird auf Wunsch direkt dort eingefügt, wo der Cursor steht.

---

## Neu in 2.0

- **Komplett neu in Rust/Tauri:** Setup rund 19 MB statt 91 MB, installiert rund 80 MB statt 380 MB.
- **Lokale Spracherkennung fest eingebaut** (whisper.cpp), keine Erweiterung mehr nötig. Die Grafikkarte wird über **Vulkan** genutzt – NVIDIA, AMD und Intel, ohne CUDA. Ohne passende Grafikkarte rechnet der Prozessor.
- **Linux mit Wayland:** Tastenkürzel und automatisches Einfügen funktionieren jetzt auch unter Wayland (siehe [Linux und Wayland](#linux-und-wayland)).
- **Mikrofonwahl** wird stabil über die Geräte-ID gespeichert. Wer in 1.0.8 ein bestimmtes Mikrofon gewählt hatte, muss es nach dem Update einmal neu auswählen.
- **Stille wird verworfen:** Ergibt eine Aufnahme nur Satzzeichen (z. B. ein einzelnes „Komma“), erscheint „Keine Sprache erkannt.“ und das Fenster schließt sich nach 2 Sekunden.
- Dampf ablassen hält die Nachricht jetzt kurz (Zusatz in der Standard-Anweisung).

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

1. `Strg+Alt+K` oder Tray-Symbol → **„Letzten Text korrigieren …“**
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
| **Groq Whisper** *(Standard, kostenlos)* | Groq API-Key |
| **OpenAI Whisper API** | OpenAI API-Key |
| **AssemblyAI** | AssemblyAI API-Key |
| **OpenRouter** (Whisper-Modelle) | OpenRouter API-Key |
| **Lokal – whisper.cpp (offline)** | nichts, das Modell wird beim ersten Diktat einmalig heruntergeladen |
| **Eigener Endpunkt** | OpenAI-kompatibler Endpunkt |

### Sprachmodell (Textverarbeitung)
| Anbieter | Was du brauchst |
|---|---|
| **Groq** *(Standard, kostenlos)* | Groq API-Key |
| **OpenAI** | OpenAI API-Key |
| **Claude (Anthropic)** | Anthropic API-Key |
| **OpenRouter** (alle Modelle) | OpenRouter API-Key |
| **Lokal – Ollama (eigenes Programm)** | [Ollama](https://ollama.com) installiert und gestartet, mindestens ein Modell geladen (`ollama pull <Modellname>`), erreichbar unter localhost:11434 |
| **Eigener Endpunkt** | OpenAI-kompatibler Endpunkt |

Jeden Workflow kannst du **unabhängig** konfigurieren – z. B. BlitzVox mit Groq Whisper, BlitzVox+ mit lokalem Ollama.

> ⚠️ Bei Groq: Die Modellauswahl in den Einstellungen ist eine editierbare Vorschlagsliste. Groq nimmt Modelle regelmäßig aus dem Angebot – schlägt ein Modell mit „404 does not exist“ fehl, ein anderes aus der Liste wählen bzw. auf console.groq.com/docs/models nachsehen, welches für deinen Tarif verfügbar ist.

### Lokale Spracherkennung
Unter Einstellungen → Workflows bzw. im Einrichtungsassistenten „Lokal – whisper.cpp (offline)“ und eine Modellgröße wählen (`tiny`, `base`, `small`, `medium`; 78 MB bis 1,5 GB). Das Modell wird beim ersten Diktat einmalig heruntergeladen und liegt danach unter `%LOCALAPPDATA%\blitzvox\models` (Windows) bzw. `~/.local/share/blitzvox/models` (Linux). Die früheren faster-whisper-Modelle aus 1.0.8 sind nicht kompatibel und können gelöscht werden.

---

## Installation

### Windows 11
1. `BlitzVox_2.0.0_x64-setup.exe` aus den [Releases](https://github.com/Hugomaulwurf76/blitzvox/releases) herunterladen und ausführen.
2. Windows fragt nach Administratorrechten. Die Installation erfolgt für alle Benutzer nach `C:\Program Files\BlitzVox`, mit Startmenü-Eintrag. Eine Desktop-Verknüpfung lässt sich auf der letzten Seite des Setups anhaken.
3. Ist **BlitzVox 1.0.8** installiert, entfernt das Setup es automatisch. Einstellungen, API-Keys, Korrekturen und Autostart bleiben erhalten.
4. Fehlt auf dem PC die Vulkan-Laufzeit (`vulkan-1.dll`, kommt normalerweise mit dem Grafiktreiber), installiert das Setup die offizielle Vulkan-Laufzeit von LunarG mit.

Das Setup ist nicht code-signiert. Windows SmartScreen oder ein Virenscanner können deshalb warnen: „Weitere Informationen“ → „Trotzdem ausführen“.

### Linux (Debian, Ubuntu, Linux Mint, Zorin OS; 64 Bit)
`blitzvox_2.0.0_amd64.deb` aus den [Releases](https://github.com/Hugomaulwurf76/blitzvox/releases) herunterladen und im Terminal installieren:
```bash
sudo apt install ./blitzvox_2.0.0_amd64.deb
```
Danach im Anwendungsmenü „BlitzVox“ starten. Deinstallation: `sudo apt remove blitzvox`.

Update von 1.0.8: Das Paket ersetzt `blitzvox` 1.0.8 und entfernt die nicht mehr benötigte Erweiterung `blitzvox-lokal`. Installiere dafür im Terminal wie oben – grafische Installationsprogramme für einzelne .deb-Dateien entfernen andere Pakete oft nicht und brechen dann ab. In dem Fall vorher `sudo apt remove blitzvox-lokal blitzvox` ausführen; Einstellungen und Keys bleiben erhalten.

### Linux und Wayland
- **X11:** Tastenkürzel über die X11-Erweiterung RECORD, Einfügen über `xdotool` (wird mitinstalliert).
- **Wayland:** BlitzVox liest die Tastatur direkt (`/dev/input`) und fügt über eine eigene virtuelle Tastatur (`/dev/uinput`) ein. Dafür richtet das Paket eine udev-Regel ein (`/usr/lib/udev/rules.d/70-blitzvox.rules`), die dem angemeldeten Benutzer Zugriff auf Tastaturen und `/dev/uinput` gibt. **Hinweis:** Damit kann jedes Programm dieses Benutzers Tastendrücke mitlesen – wie bei ydotool und ähnlichen Werkzeugen. Wer das nicht möchte, kann die Regel löschen; BlitzVox nutzt dann, falls vorhanden, die Kürzel-Schnittstelle des Desktops (Portal „GlobalShortcuts“, z. B. KDE Plasma 6, GNOME ab 48), sonst wirken die Kürzel nur in X11-Programmen.

---

## Erster Start

1. BlitzVox über das Startmenü bzw. Anwendungsmenü starten.
2. Der **Einrichtungsassistent** führt durch Sprache, Spracherkennung, KI-Funktionen, Mikrofon, Tastenkürzel und ein Probediktat. Du kannst ihn jederzeit unter Einstellungen → Erweitert erneut aufrufen.
3. Das Tray-Symbol erscheint in der Taskleiste. Windows 11 legt neue Symbole oft im Überlauf (`^`) ab; unter „Einstellungen → Personalisierung → Taskleiste → Andere Taskleistensymbole“ lässt es sich dauerhaft einblenden.
4. Diktieren: Tastenkürzel drücken (Standard `Strg+Alt+W`) oder über das Tray-Menü.

Autostart: Einstellungen → Erweitert → „Mit Windows starten“ bzw. „Beim Anmelden starten“.

---

## Konfiguration

| | Windows | Linux |
|---|---|---|
| Einstellungen | `%APPDATA%\blitzvox\settings.json` | `~/.config/blitzvox/settings.json` |
| Gelernte Korrekturen | `%APPDATA%\blitzvox\corrections.json` | `~/.config/blitzvox/corrections.json` |
| Fehlerprotokoll | `%APPDATA%\blitzvox\error.log` | `~/.config/blitzvox/error.log` |
| Modelle (lokal) | `%LOCALAPPDATA%\blitzvox\models` | `~/.local/share/blitzvox/models` |

API-Keys liegen im Schlüsselbund des Systems (Windows-Anmeldeinformationen bzw. Secret Service), nicht in der `settings.json`. Das Format ist dasselbe wie in 1.0.8.

---

## Selbst bauen (Entwickler)

### Windows
Voraussetzungen: [Rust](https://rustup.rs) (MSVC), Visual Studio 2022 mit „Desktopentwicklung mit C++“ und C++-CMake-Tools, LLVM/Clang (`LIBCLANG_PATH` gesetzt), [Vulkan SDK](https://vulkan.lunarg.com) (`VULKAN_SDK` gesetzt) und `cargo install tauri-cli --version 2.12.6 --locked`.

```cmd
dev.cmd     – Entwicklungsversion starten
build.cmd   – Setup bauen (target\release\bundle\nsis\)
```

whisper.cpp scheitert unter Windows an zu langen Build-Pfaden. Die Skripte bauen deshalb nach `D:\bv`, einer Verknüpfung auf `target` (`mklink /J D:\bv <Projektordner>\target`); bei Bedarf in den Skripten anpassen.

### Linux
Die nötigen Pakete und Befehle stehen am Anfang von `build-linux.sh`. Danach:
```bash
bash build-linux.sh
```
Das Paket liegt anschließend in `src-tauri/target/release/bundle/deb/`.

### Tests
```bash
cd src-tauri
cargo test --lib
```
Die Testdaten in `src-tauri/tests/data` wurden mit den Skripten in `tools/` aus der Python-Version 1.0.8 erzeugt (Satzzeichen-Regeln und Prompts sind wortgleich).

---

## Projektstruktur

```
ui/                        Oberfläche (HTML/CSS/JS ohne Build-Schritt)
  workflow.*               Aufnahmefenster mit Wellenform
  settings.*               Einstellungen
  wizard.*                 Einrichtungsassistent
  correction.*             „Letzten Text korrigieren“
  keys.js                  Tastenkürzel, wenn ein BlitzVox-Fenster den Fokus hat
src-tauri/
  src/app.rs               Programmrahmen: Tray, Fenster, Befehle
  src/workflow.rs          Kernlogik aller vier Workflows
  src/settings.rs          Einstellungen (kompatibel zu 1.0.8)
  src/secrets.rs           API-Keys im System-Schlüsselbund
  src/recorder.rs          Audioaufnahme
  src/stt.rs, local_stt.rs Spracherkennung (Cloud, lokal mit whisper.cpp)
  src/llm.rs               Sprachmodelle
  src/corrections.rs       Satzzeichen-Befehle, Korrektur-Wörterbuch, Lernlogik
  src/hotkeys.rs           Globale Tastenkürzel (Windows-Hook, X11, Wayland)
  src/linux_input.rs       Wayland: Tastatur lesen, Portal, virtuelle Tastatur
  src/paste.rs             Zwischenablage und automatisches Einfügen
  src/autostart.rs         Autostart (Registry bzw. ~/.config/autostart)
  windows/hooks.nsh        Setup: 1.0.8 entfernen, Vulkan-Laufzeit
  linux/                   .deb: Menüeintrag, udev-Regel, Paketskripte
tools/                     Erzeugen der Testdaten aus 1.0.8
```

---

## Datenfluss

```
Online (Groq/OpenAI/Claude/…):  Dein PC → gewählter API-Anbieter
Lokal (whisper.cpp/Ollama):     läuft komplett auf deiner eigenen Hardware
```

Kein eigenes Backend. Audio und Text gehen direkt vom PC zum gewählten Anbieter.

---

## Lizenz

MIT License – abgeleitet vom Original [cmagnussen/blitztext-app](https://github.com/cmagnussen/blitztext-app).
