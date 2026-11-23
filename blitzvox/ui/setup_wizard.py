"""
Einrichtungsassistent — erscheint beim ersten Start und lässt sich
im Tab „Erweitert“ jederzeit erneut aufrufen.

Seiten:
  1. Willkommen + Diktiersprache
  2. Spracherkennung + API-Key (mit Key-Test)
  3. KI-Funktionen (optional)
  4. Mikrofon (mit Pegelanzeige)
  5. Tastenkürzel
  6. Probediktat (echte Aufnahme + Transkription mit den gewählten Werten)
  7. Abschluss (Automatisch einfügen, Autostart, Zusammenfassung)

Der Assistent arbeitet auf einer Kopie der Einstellungen und gibt sie erst
beim Klick auf „Fertig“ über das Signal `completed` zurück.
Ausnahme: API-Keys werden beim Weiterklicken im Schlüsselbund gespeichert,
weil das Probediktat sie bereits braucht.
"""

import asyncio
import copy
import os
import platform
import shutil
import sys
import threading
from typing import Optional

from PyQt6.QtCore import Qt, QObject, QTimer, pyqtSignal
from PyQt6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QLabel, QPushButton, QStackedWidget,
    QWidget, QFrame, QComboBox, QFormLayout, QLineEdit, QCheckBox,
    QProgressBar, QTextEdit, QScrollArea, QApplication, QGridLayout,
)

from .theme import set_prop, T
from .. import __version__
from ..services.settings import (
    AppSettings, STTProvider, LLMProvider, LLM_MODEL_PRESETS,
    get_api_key, set_api_key, masked,
    KEY_GROQ, KEY_OPENAI, KEY_ASSEMBLYAI, KEY_OPENROUTER, KEY_ANTHROPIC,
    KEY_CUSTOM_STT,
)


# ── Wann muss der Assistent automatisch starten? ─────────────────────────────

# STT-Anbieter, die zwingend einen API-Key brauchen.
# LOCAL braucht keinen; CUSTOM braucht eine URL (Key ist dort optional).
_STT_KEY_REQUIRED = {
    STTProvider.GROQ:       KEY_GROQ,
    STTProvider.OPENAI:     KEY_OPENAI,
    STTProvider.ASSEMBLYAI: KEY_ASSEMBLYAI,
    STTProvider.OPENROUTER: KEY_OPENROUTER,
}

# LLM-Anbieter, die einen API-Key brauchen
_LLM_KEY_REQUIRED = {
    LLMProvider.GROQ:       KEY_GROQ,
    LLMProvider.OPENAI:     KEY_OPENAI,
    LLMProvider.ANTHROPIC:  KEY_ANTHROPIC,
    LLMProvider.OPENROUTER: KEY_OPENROUTER,
}

# Wo man den Key bekommt
_KEY_LINKS = {
    KEY_GROQ:       ("Groq (kostenlos)", "https://console.groq.com"),
    KEY_OPENAI:     ("OpenAI", "https://platform.openai.com"),
    KEY_ASSEMBLYAI: ("AssemblyAI", "https://www.assemblyai.com"),
    KEY_OPENROUTER: ("OpenRouter", "https://openrouter.ai"),
    KEY_ANTHROPIC:  ("Anthropic", "https://console.anthropic.com"),
}

_KEY_PLACEHOLDERS = {
    KEY_GROQ: "gsk_…", KEY_OPENAI: "sk-…", KEY_ASSEMBLYAI: "…",
    KEY_OPENROUTER: "sk-or-v1-…", KEY_ANTHROPIC: "sk-ant-…",
}

# Standard-STT-Modell je Anbieter (leer = Vorgabe aus services/stt.py)
_STT_DEFAULT_MODEL = {
    STTProvider.GROQ:   "whisper-large-v3-turbo",
    STTProvider.OPENAI: "whisper-1",
}

_LOCAL_SIZES = [
    ("Sehr klein (tiny) – am schnellsten, ungenau", "tiny"),
    ("Klein (base) – Standard", "base"),
    ("Mittel (small) – genauer, langsamer", "small"),
    ("Groß (medium) – sehr genau, deutlich langsamer", "medium"),
]


def needs_setup(settings: AppSettings) -> bool:
    """True, wenn der Assistent beim Start automatisch erscheinen soll:
    - beim allerersten Start (has_seen_onboarding ist False), oder
    - wenn die Spracherkennung nicht einsatzbereit ist (Key bzw. URL fehlt).
    """
    if not settings.has_seen_onboarding:
        return True

    provider = settings.transcription_model.stt_provider
    if provider in _STT_KEY_REQUIRED:
        return get_api_key(_STT_KEY_REQUIRED[provider]) is None
    if provider == STTProvider.CUSTOM:
        url = settings.transcription_model.custom_stt_url or settings.custom_stt_url
        return not url.strip()
    return False


# ── Hilfsfunktionen ──────────────────────────────────────────────────────────

#: Auswahl für die Diktiersprache (Anzeige, Sprachcode). Leerer Code = automatisch erkennen.
LANGUAGES = [
    ("Deutsch", "de"),
    ("Englisch", "en"),
    ("Französisch", "fr"),
    ("Spanisch", "es"),
    ("Italienisch", "it"),
    ("Niederländisch", "nl"),
    ("Polnisch", "pl"),
    ("Portugiesisch", "pt"),
    ("Türkisch", "tr"),
    ("Automatisch erkennen", ""),
]


def _workflow_models(settings: AppSettings) -> list:
    """Alle Modell-Konfigurationen der vier Workflows."""
    return [
        settings.transcription_model,
        settings.text_improvement.model,
        settings.dampf_ablassen.model,
        settings.emoji.model,
    ]


def _llm_models(settings: AppSettings) -> list:
    """Modell-Konfigurationen der drei Workflows mit Sprachmodell."""
    return [
        settings.text_improvement.model,
        settings.dampf_ablassen.model,
        settings.emoji.model,
    ]


_KEY_LABELS = {"ctrl": "Strg", "alt": "Alt", "shift": "Umschalt", "cmd": "Win",
               "meta": "Win", "space": "Leertaste"}


def format_hotkey(combo: str) -> str:
    """'ctrl+alt+w' -> 'Strg+Alt+W'"""
    parts = []
    for p in (combo or "").split("+"):
        p = p.strip().lower()
        if not p:
            continue
        parts.append(_KEY_LABELS.get(p, p.upper() if len(p) == 1 else p.capitalize()))
    return "+".join(parts) or "—"


def _session_is_wayland() -> bool:
    return (platform.system() == "Linux"
            and os.environ.get("XDG_SESSION_TYPE", "").lower() == "wayland")


def check_api_key(key_name: str, key: str) -> tuple:
    """Prüft einen API-Key mit einer reinen Leseanfrage beim Anbieter.

    Liefert (status, meldung) mit status:
      "ok"          – Key gültig
      "invalid"     – Key abgelehnt (401/403)
      "unreachable" – Anbieter nicht erreichbar, Key ungeprüft
    """
    try:
        import httpx
    except ImportError:
        return "unreachable", "httpx fehlt, der Key kann nicht geprüft werden."

    if key_name == KEY_GROQ:
        url, headers = "https://api.groq.com/openai/v1/models", {"Authorization": f"Bearer {key}"}
    elif key_name == KEY_OPENAI:
        url, headers = "https://api.openai.com/v1/models", {"Authorization": f"Bearer {key}"}
    elif key_name == KEY_OPENROUTER:
        url, headers = "https://openrouter.ai/api/v1/key", {"Authorization": f"Bearer {key}"}
    elif key_name == KEY_ASSEMBLYAI:
        url, headers = "https://api.assemblyai.com/v2/transcript?limit=1", {"authorization": key}
    elif key_name == KEY_ANTHROPIC:
        url, headers = "https://api.anthropic.com/v1/models", {
            "x-api-key": key, "anthropic-version": "2023-06-01"}
    else:
        return "unreachable", "Für diesen Anbieter gibt es keinen Key-Test."

    try:
        resp = httpx.get(url, headers=headers, timeout=10)
    except Exception as exc:
        return "unreachable", f"Anbieter nicht erreichbar ({type(exc).__name__})."

    if resp.status_code == 200:
        return "ok", "Key ist gültig."
    if resp.status_code in (401, 403):
        return "invalid", "Der Key wurde abgelehnt. Bitte prüfen und neu einfügen."
    return "unreachable", f"Unerwartete Antwort vom Anbieter (Fehler {resp.status_code})."


def check_ollama(base_url: str) -> tuple:
    """Prüft, ob Ollama unter base_url läuft. Liefert (status, meldung, modelle),
    wobei modelle die in Ollama wirklich installierten Modelle sind."""
    try:
        import httpx
        resp = httpx.get(base_url.rstrip("/") + "/api/tags", timeout=5)
    except Exception:
        return "invalid", ("Ollama ist unter dieser Adresse nicht erreichbar. Ist Ollama "
                           "installiert und gestartet?"), []
    if resp.status_code != 200:
        return "invalid", f"Ollama antwortet mit Fehler {resp.status_code}.", []
    try:
        names = sorted(m.get("name", "") for m in resp.json().get("models", []) if m.get("name"))
    except Exception:
        names = []
    if not names:
        return "empty", ("Ollama läuft, aber es ist noch kein Modell installiert. Lade in "
                         "Ollama ein Modell herunter („ollama pull <Modellname>“) und teste "
                         "dann erneut."), []
    return "ok", f"Ollama läuft. Installierte Modelle: {len(names)}.", names


class _ResultBridge(QObject):
    """Überträgt Ergebnisse aus Hintergrund-Threads in den Qt-Hauptthread."""
    result = pyqtSignal(object)


_bridges: set = set()   # hält laufende Brücken am Leben


def _run_in_thread(func, callback) -> None:
    """func() im Hintergrund ausführen, callback(ergebnis) im Hauptthread aufrufen."""
    bridge = _ResultBridge()
    _bridges.add(bridge)

    def deliver(res):
        _bridges.discard(bridge)
        callback(res)

    bridge.result.connect(deliver)

    def target():
        try:
            res = func()
        except Exception as exc:
            res = ("unreachable", str(exc))
        bridge.result.emit(res)

    threading.Thread(target=target, daemon=True).start()


def _hint(text: str) -> QLabel:
    lbl = QLabel(text)
    set_prop(lbl, "role", "hint")
    lbl.setWordWrap(True)
    return lbl


def _clear_wizard_error(widget: QWidget) -> None:
    """Rote Fehlerzeile des Assistenten ausblenden, sobald das Problem behoben ist."""
    win = widget.window()
    if hasattr(win, "clear_error"):
        win.clear_error()


def _overview_label() -> QLabel:
    """Zeile „Eingerichtet: …“ über der Dienst-Auswahl."""
    lbl = QLabel()
    lbl.setWordWrap(True)
    set_prop(lbl, "role", "strong")
    # helleres Grün als T["success"]; auf dem hellen Hintergrund noch gut lesbar
    lbl.setStyleSheet("color: #16a34a;")
    return lbl


def _set_overview(label: QLabel, names: list) -> None:
    label.setText("Eingerichtet: " + ", ".join(names) if names
                  else "Eingerichtet: noch keiner")


def _info_box(title: str, text: str) -> QFrame:
    """Hinweiskasten: weißer Hintergrund, Rahmen, runde Ecken."""
    box = QFrame()
    box.setObjectName("wizardInfoBox")
    box.setStyleSheet(
        f"QFrame#wizardInfoBox {{ background: {T['surface-card']}; "
        f"border: 1px solid {T['hairline-strong']}; border-radius: 10px; }}"
        "QFrame#wizardInfoBox QLabel { background: transparent; border: none; }"
    )
    layout = QVBoxLayout(box)
    layout.setContentsMargins(16, 12, 16, 14)
    layout.setSpacing(6)
    head = QLabel(title)
    set_prop(head, "role", "strong")
    layout.addWidget(head)
    lbl = QLabel(text)
    lbl.setWordWrap(True)
    layout.addWidget(lbl)
    return box


class _CenteredBox(QWidget):
    """Zeigt ein Widget zentriert mit höchstens 80 % der Fensterbreite."""

    def __init__(self, inner: QWidget, ratio: float = 0.8, parent=None):
        super().__init__(parent)
        self._inner = inner
        self._ratio = ratio
        row = QHBoxLayout(self)
        row.setContentsMargins(0, 0, 0, 0)
        row.addStretch(1)
        row.addWidget(inner)
        row.addStretch(1)

    def resizeEvent(self, event) -> None:
        target = int(self.window().width() * self._ratio)
        self._inner.setFixedWidth(max(200, min(target, self.width())))
        super().resizeEvent(event)


RELEASES_URL = "https://github.com/Hugomaulwurf76/blitzvox/releases"

GPU_HINT = (
    "Mit einer NVIDIA-Grafikkarte rechnet die lokale Erkennung deutlich schneller. "
    "Dafür brauchst du eine NVIDIA-Grafikkarte, einen aktuellen NVIDIA-Treiber sowie "
    'CUDA 12 (<a href="https://developer.nvidia.com/cuda-downloads">developer.nvidia.com/cuda-downloads</a>) '
    'und cuDNN 9 (<a href="https://developer.nvidia.com/cudnn">developer.nvidia.com/cudnn</a>). '
    "Ist das vorhanden, nutzt BlitzVox die Grafikkarte automatisch, sonst den Prozessor. "
    "AMD- und Intel-Grafikkarten werden nicht unterstützt."
)


def local_help_items() -> list:
    """Inhalt des Fensters „Lokale Spracherkennung ist nicht installiert“, je nach Installation."""
    from ..services import local_addon
    mode = local_addon.install_mode()
    items: list = []
    if mode == "windows":
        items += [
            ("text", "Die lokale Spracherkennung ist eine Erweiterung, die du bei der "
                     "Installation auswählen kannst. So richtest du sie ein:"),
            ("text", "1. BlitzVox beenden: Rechtsklick auf das Tray-Symbol → „Beenden“.<br>"
                     "2. Den BlitzVox-Installer erneut ausführen "
                     f'(Download: <a href="{RELEASES_URL}">GitHub-Releases</a>).<br>'
                     "3. Bei „Komponenten auswählen“ den Haken bei "
                     "„Lokale Spracherkennung“ setzen.<br>"
                     "4. BlitzVox wieder starten."),
        ]
    elif mode == "linux":
        items += [
            ("text", "Die lokale Spracherkennung ist ein eigenes Zusatzpaket. So richtest du sie ein:"),
            ("text", "1. <b>blitzvox-lokal_" + __version__ + "_amd64.deb</b> von den "
                     f'<a href="{RELEASES_URL}">GitHub-Releases</a> herunterladen.<br>'
                     "2. Im Terminal im Download-Ordner installieren:"),
            ("code", f"sudo apt install ./blitzvox-lokal_{__version__}_amd64.deb"),
            ("text", "3. BlitzVox beenden und wieder starten."),
        ]
    else:
        items += [
            ("text", "Du startest BlitzVox aus dem Quellcode. Installiere faster-whisper in "
                     "das Python, mit dem BlitzVox läuft:"),
            ("code", f'"{sys.executable}" -m pip install faster-whisper'),
            ("text", "Danach BlitzVox beenden und wieder starten."),
        ]
    err = local_addon.load_error()
    if err and local_addon.addon_dir().is_dir():
        items.append(("text", f"<i>Die Erweiterung ist vorhanden, lässt sich aber nicht laden: {err}</i>"))
    items.append(("text", "Bis dahin kannst du einen Online-Dienst wählen, z. B. Groq (kostenlos)."))
    items.append(("box", "Grafikkarte (GPU) nutzen", GPU_HINT))
    return items


def show_local_help(parent=None) -> None:
    from .key_popup import show_info
    show_info("Lokale Spracherkennung ist nicht installiert", local_help_items(), parent)


def _status(label: QLabel, role: str, text: str) -> None:
    set_prop(label, "role", role)
    label.setText(text)
    label.setVisible(bool(text))


# ── Baustein: API-Key-Feld mit Test ──────────────────────────────────────────

class KeyBox(QWidget):
    """Eingabe für einen API-Key: Feld, Anzeigen, Einfügen, Testen, Status.
    Ein erfolgreich getesteter Key wird sofort gespeichert. So kann man mehrere
    Dienste nacheinander einrichten, ohne dass beim Wechsel etwas verloren geht."""

    key_saved = pyqtSignal(str)   # key_name

    def __init__(self, parent=None):
        super().__init__(parent)
        self.key_name: Optional[str] = None
        self._verified: Optional[tuple] = None   # (key_name, key) nach erfolgreichem Test
        self._token = 0

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(6)

        self._link = QLabel()
        self._link.setOpenExternalLinks(True)
        self._link.setTextInteractionFlags(Qt.TextInteractionFlag.TextBrowserInteraction)
        layout.addWidget(self._link)

        self._stored = _hint("")
        layout.addWidget(self._stored)

        row = QHBoxLayout()
        row.setSpacing(8)
        self.field = QLineEdit()
        self.field.setEchoMode(QLineEdit.EchoMode.Password)
        self.field.textChanged.connect(self._on_text_changed)
        row.addWidget(self.field, 1)

        self._show_btn = QPushButton("Anzeigen")
        self._show_btn.setToolTip("Key vollständig in einem eigenen Fenster anzeigen")
        self._show_btn.clicked.connect(self._show_key)
        row.addWidget(self._show_btn)

        paste = QPushButton("Einfügen")
        paste.clicked.connect(self._paste)
        row.addWidget(paste)

        self._test_btn = QPushButton("Key testen")
        self._test_btn.clicked.connect(self.test)
        row.addWidget(self._test_btn)
        layout.addLayout(row)

        self._status = QLabel()
        self._status.setWordWrap(True)
        self._status.hide()
        layout.addWidget(self._status)

    def set_key_name(self, key_name: str) -> None:
        if key_name == self.key_name:
            return
        self.key_name = key_name
        self._verified = None
        self._token += 1
        self.field.clear()
        name, url = _KEY_LINKS.get(key_name, ("", ""))
        self._link.setText(f'Key erstellen bei {name}: <a href="{url}">{url}</a>')
        self.field.setPlaceholderText(_KEY_PLACEHOLDERS.get(key_name, ""))
        self._show_stored()
        _status(self._status, "hint", "")
        self._test_btn.setEnabled(True)

    def _show_stored(self) -> None:
        stored = get_api_key(self.key_name)
        if stored:
            self._stored.setText(f"Gespeichert: {masked(stored)}. "
                                 "Feld leer lassen, um diesen Key zu behalten.")
        else:
            self._stored.setText("Noch kein Key gespeichert.")
        self._update_show_btn()

    def _show_key(self) -> None:
        """Eingetippten Key, sonst den gespeicherten, im eigenen Fenster zeigen."""
        key = self.field.text().strip() or (get_api_key(self.key_name) or "")
        if not key:
            return
        from .key_popup import show_key
        name = _KEY_LINKS.get(self.key_name, ("", ""))[0].split(" (")[0]
        show_key(f"{name} API-Key", key, self)

    def _update_show_btn(self) -> None:
        has_key = bool(self.field.text().strip()) or bool(get_api_key(self.key_name))
        self._show_btn.setEnabled(has_key)

    def _paste(self) -> None:
        text = QApplication.clipboard().text().strip()
        if text:
            self.field.setText(text)

    def _on_text_changed(self) -> None:
        self._verified = None
        _status(self._status, "hint", "")
        self._update_show_btn()

    def test(self) -> None:
        key = self.field.text().strip() or (get_api_key(self.key_name) or "")
        if not key:
            _status(self._status, "error", "Bitte zuerst einen Key eintragen.")
            return
        self._token += 1
        token, key_name = self._token, self.key_name
        self._test_btn.setEnabled(False)
        _status(self._status, "hint", "Key wird geprüft …")
        _run_in_thread(lambda: check_api_key(key_name, key),
                       lambda res: self._on_result(token, key_name, key, res))

    def _on_result(self, token, key_name, key, res) -> None:
        if token != self._token:
            return  # veraltetes Ergebnis (Key oder Anbieter inzwischen geändert)
        self._test_btn.setEnabled(True)
        status, message = res
        if status == "ok":
            ok, error = self._save(key_name, key)
            if not ok:
                _status(self._status, "error", error)
                return
            _status(self._status, "success", "✓ " + message + " Gespeichert.")
            _clear_wizard_error(self)
        elif status == "unreachable":
            # Key lässt sich nicht prüfen (z. B. offline) -> trotzdem zulassen
            self._verified = (key_name, key)
            _status(self._status, "hint", message + " Der Key wird ungeprüft übernommen.")
        else:
            _status(self._status, "error", message)

    def _save(self, key_name: str, key: str) -> tuple:
        """Key im Schlüsselbund speichern, Feld leeren. Liefert (ok, Fehlermeldung)."""
        try:
            set_api_key(key_name, key)
        except Exception as exc:
            return False, ("Der Key konnte nicht im Schlüsselbund des Systems "
                           f"gespeichert werden: {exc}")
        self.field.blockSignals(True)
        self.field.clear()
        self.field.blockSignals(False)
        self._verified = None
        self._show_stored()
        self.key_saved.emit(key_name)
        return True, ""

    def commit(self) -> tuple:
        """Für „Weiter“: Key muss getestet sein. Liefert (ok, Fehlermeldung)."""
        typed = self.field.text().strip()
        if typed:
            if self._verified != (self.key_name, typed):
                return False, "Bitte den Key zuerst mit „Key testen“ prüfen."
            # nur bei „Anbieter nicht erreichbar“: ungeprüft übernehmen
            return self._save(self.key_name, typed)
        if get_api_key(self.key_name):
            return True, ""
        return False, "Bitte einen API-Key eintragen und mit „Key testen“ prüfen."


# ── Seiten ───────────────────────────────────────────────────────────────────

class WizardPage(QWidget):
    """Basisklasse für eine Seite. Unterklassen füllen `body` mit Inhalt."""

    def __init__(self, title: str, lead: str, parent=None):
        super().__init__(parent)
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        outer.addWidget(scroll)

        inner = QWidget()
        scroll.setWidget(inner)
        layout = QVBoxLayout(inner)
        layout.setContentsMargins(32, 28, 32, 16)
        layout.setSpacing(12)

        title_label = QLabel(title)
        set_prop(title_label, "role", "display-sm")
        title_label.setWordWrap(True)
        layout.addWidget(title_label)

        lead_label = QLabel(lead)
        set_prop(lead_label, "role", "lead")
        lead_label.setWordWrap(True)
        layout.addWidget(lead_label)

        self.body = QVBoxLayout()
        self.body.setSpacing(10)
        layout.addSpacing(6)
        layout.addLayout(self.body)
        layout.addStretch(1)

    def load(self, settings: AppSettings) -> None:
        """Werte aus den Einstellungen in die Felder übernehmen (einmal beim Start)."""

    def enter(self, settings: AppSettings) -> None:
        """Seite wird angezeigt."""

    def leave(self) -> None:
        """Seite wird verlassen oder der Assistent geschlossen."""

    def store(self, settings: AppSettings) -> None:
        """Werte aus den Feldern in die Einstellungen schreiben."""

    def can_continue(self) -> tuple:
        """Prüft, ob „Weiter“ erlaubt ist. Liefert (ok, Fehlermeldung)."""
        return True, ""


# 1 ── Willkommen ──────────────────────────────────────────────────────────────

class WelcomePage(WizardPage):
    def __init__(self, parent=None):
        super().__init__(
            "Willkommen bei BlitzVox",
            "BlitzVox wandelt Gesprochenes per Tastenkürzel in Text um und fügt ihn "
            "dort ein, wo dein Cursor steht. In wenigen Schritten richten wir alles ein.",
            parent,
        )
        form = QFormLayout()
        form.setLabelAlignment(Qt.AlignmentFlag.AlignLeft)
        self._language = QComboBox()
        for name, code in LANGUAGES:
            self._language.addItem(name, code)
        form.addRow("In welcher Sprache diktierst du?", self._language)
        self.body.addLayout(form)
        self.body.addWidget(_hint(
            "Eine feste Sprache liefert die genauesten Ergebnisse. "
            "„Automatisch erkennen“ ist sinnvoll, wenn du oft die Sprache wechselst. "
            "Weitere Sprachen kannst du später in den Einstellungen unter „Workflows“ eintragen."
        ))
        self.body.addSpacing(8)
        self.body.addWidget(_CenteredBox(_info_box(
            "Was passiert, wenn du abbrichst?",
            "Brichst du jetzt hier in Schritt 1 ab, wird nichts gespeichert. Brichst du "
            "später ab, bleibt alles gespeichert, was du bis dahin mit „Weiter“ bestätigt hast.")))

    def load(self, settings: AppSettings) -> None:
        code = settings.transcription_model.language or ""
        idx = self._language.findData(code)
        if idx < 0:
            # Sprache wurde vorher in den Einstellungen von Hand eingetragen
            self._language.addItem(f"Eigene Einstellung ({code})", code)
            idx = self._language.count() - 1
        self._language.setCurrentIndex(idx)

    def store(self, settings: AppSettings) -> None:
        code = self._language.currentData() or ""
        for model in _workflow_models(settings):
            model.language = code


# 2 ── Spracherkennung ─────────────────────────────────────────────────────────

class SpeechPage(WizardPage):
    def __init__(self, parent=None):
        super().__init__(
            "Spracherkennung",
            "Wähle, welcher Dienst deine Sprache in Text umwandelt. "
            "Groq ist kostenlos und für den Start empfohlen.",
            parent,
        )
        self._initial_provider = None
        from ..services import local_addon
        self._local_available = local_addon.is_available()
        self._loading = True   # beim ersten Füllen kein Hinweisfenster öffnen

        self._overview = _overview_label()
        self.body.addWidget(self._overview)

        form = QFormLayout()
        self._provider = QComboBox()
        for val in STTProvider.ALL:
            self._provider.addItem(STTProvider.DISPLAY[val], val)
        self._provider.currentIndexChanged.connect(self._update_mode)
        form.addRow("Dienst:", self._provider)
        self.body.addLayout(form)

        # Bereich: API-Key
        self._key_box = KeyBox()
        self._key_box.key_saved.connect(lambda _: self._update_overview())
        self.body.addWidget(self._key_box)

        # Bereich: lokal
        self._local_box = QWidget()
        ll = QVBoxLayout(self._local_box)
        ll.setContentsMargins(0, 0, 0, 0)
        lf = QFormLayout()
        self._local_size = QComboBox()
        for name, size in _LOCAL_SIZES:
            self._local_size.addItem(name, size)
        lf.addRow("Modellgröße:", self._local_size)
        ll.addLayout(lf)
        ll.addWidget(_hint(
            "Läuft komplett auf deinem Rechner, ohne Internet und ohne Key. "
            "Beim ersten Diktat wird das gewählte Modell einmalig heruntergeladen."))
        miss_row = QHBoxLayout()
        self._local_missing = QLabel("Die lokale Erkennung ist nicht installiert.")
        set_prop(self._local_missing, "role", "error")
        self._local_missing.setWordWrap(True)
        miss_row.addWidget(self._local_missing, 1)
        self._local_help_btn = QPushButton("So richtest du sie ein")
        self._local_help_btn.clicked.connect(lambda: show_local_help(self))
        miss_row.addWidget(self._local_help_btn)
        ll.addLayout(miss_row)
        self._local_gpu = _hint("")
        ll.addWidget(self._local_gpu)
        self.body.addWidget(self._local_box)

        # Bereich: eigener Endpunkt
        self._custom_box = QWidget()
        cl = QFormLayout(self._custom_box)
        cl.setContentsMargins(0, 0, 0, 0)
        self._custom_url = QLineEdit()
        self._custom_url.setPlaceholderText("https://…/v1/audio/transcriptions")
        self._custom_key = QLineEdit()
        self._custom_key.setEchoMode(QLineEdit.EchoMode.Password)
        cl.addRow("Adresse (URL):", self._custom_url)
        cl.addRow("Key (optional):", self._custom_key)
        self.body.addWidget(self._custom_box)

        self.body.addWidget(_hint(
            "Du kannst mehrere Dienste nacheinander einrichten: Dienst wählen, Key "
            "eintragen, „Key testen“ – dann den nächsten Dienst wählen. Jeder getestete "
            "Key wird sofort gespeichert. BlitzVox nutzt den Dienst, der beim Klick auf "
            "„Weiter“ ausgewählt ist. Die anderen kannst du später in den Einstellungen "
            "unter „Workflows“ jedem Workflow einzeln zuweisen."))
        self.body.addWidget(_hint(
            "API-Keys werden sicher im Schlüsselbund deines Systems gespeichert, "
            "nicht in einer Datei."))

    def _provider_value(self) -> str:
        return self._provider.currentData()

    def _update_overview(self) -> None:
        names = [STTProvider.DISPLAY[p].split(" (")[0]
                 for p, key in _STT_KEY_REQUIRED.items() if get_api_key(key)]
        _set_overview(self._overview, names)

    def _update_mode(self) -> None:
        self._update_overview()
        p = self._provider_value()
        needs_key = p in _STT_KEY_REQUIRED
        self._key_box.setVisible(needs_key)
        if needs_key:
            self._key_box.set_key_name(_STT_KEY_REQUIRED[p])
        self._local_box.setVisible(p == STTProvider.LOCAL)
        self._local_missing.setVisible(not self._local_available)
        self._local_help_btn.setVisible(not self._local_available)
        if p == STTProvider.LOCAL and self._local_available:
            from ..services import local_addon
            gpus = local_addon.cuda_device_count()
            self._local_gpu.setText(
                "NVIDIA-Grafikkarte erkannt: BlitzVox versucht, sie zu nutzen. Fehlen CUDA 12 "
                "oder cuDNN 9, rechnet automatisch der Prozessor." if gpus
                else "Keine NVIDIA-Grafikkarte erkannt: Die Erkennung läuft auf dem Prozessor. "
                     "Das belastet ihn und ist langsamer als die Online-Dienste.")
            self._local_gpu.show()
        else:
            self._local_gpu.hide()
        self._custom_box.setVisible(p == STTProvider.CUSTOM)
        if p == STTProvider.LOCAL and not self._local_available and not self._loading:
            # Fenster erst nach dem Umschalten der Auswahl öffnen
            QTimer.singleShot(0, lambda: show_local_help(self))

    def load(self, settings: AppSettings) -> None:
        cfg = settings.transcription_model
        self._initial_provider = cfg.stt_provider
        idx = self._provider.findData(cfg.stt_provider)
        self._provider.setCurrentIndex(max(idx, 0))
        size_idx = self._local_size.findData(cfg.local_model_size or "base")
        self._local_size.setCurrentIndex(size_idx if size_idx >= 0 else 1)
        self._custom_url.setText(cfg.custom_stt_url or settings.custom_stt_url)
        stored = get_api_key(KEY_CUSTOM_STT)
        self._custom_key.setPlaceholderText(
            f"Gespeichert: {masked(stored)}" if stored else "Nicht gesetzt")
        self._update_mode()
        self._loading = False

    def can_continue(self) -> tuple:
        p = self._provider_value()
        if p in _STT_KEY_REQUIRED:
            return self._key_box.commit()
        if p == STTProvider.LOCAL and not self._local_available:
            return False, ("Die lokale Erkennung ist nicht installiert. Bitte einrichten "
                           "(Knopf „So richtest du sie ein“) oder einen Online-Dienst wählen.")
        if p == STTProvider.CUSTOM:
            url = self._custom_url.text().strip()
            if not url.startswith(("http://", "https://")):
                return False, ("Bitte eine gültige Adresse eintragen "
                               "(beginnt mit http:// oder https://).")
            key = self._custom_key.text().strip()
            if key:
                try:
                    set_api_key(KEY_CUSTOM_STT, key)
                except Exception as exc:
                    return False, f"Der Key konnte nicht gespeichert werden: {exc}"
        return True, ""

    def store(self, settings: AppSettings) -> None:
        p = self._provider_value()
        url = self._custom_url.text().strip()
        for model in _workflow_models(settings):
            if p != self._initial_provider:
                # Anderer Dienst -> dessen Standardmodell, sonst eigenes Modell behalten
                model.stt_model = _STT_DEFAULT_MODEL.get(p, "")
            model.stt_provider = p
            model.local_model_size = self._local_size.currentData()
            if p == STTProvider.CUSTOM:
                model.custom_stt_url = url
        if p == STTProvider.CUSTOM:
            settings.custom_stt_url = url


# 4 ── Mikrofon ────────────────────────────────────────────────────────────────

class MicrophonePage(WizardPage):
    def __init__(self, parent=None):
        super().__init__(
            "Mikrofon",
            "Wähle dein Mikrofon und sprich kurz hinein. "
            "Der Balken zeigt, ob BlitzVox dich hört.",
            parent,
        )
        self._recorder = None
        self._peak = 0.0
        self._error: Optional[str] = None

        form = QFormLayout()
        self._device = QComboBox()
        self._device.addItem("System-Standard", None)
        self._device.currentIndexChanged.connect(self._restart_monitor)
        form.addRow("Aufnahmegerät:", self._device)
        self.body.addLayout(form)

        self._level = QProgressBar()
        self._level.setRange(0, 100)
        self._level.setTextVisible(False)
        self._level.setFixedHeight(14)
        self._level.setStyleSheet(
            f"QProgressBar {{ background: {T['surface-strong']}; border: 1px solid "
            f"{T['hairline-strong']}; border-radius: 7px; }}"
            f"QProgressBar::chunk {{ background: {T['success']}; border-radius: 6px; }}"
        )
        self.body.addWidget(self._level)

        self._status = QLabel()
        self._status.setWordWrap(True)
        self.body.addWidget(self._status)

        self._timer = QTimer(self)
        self._timer.setInterval(50)
        self._timer.timeout.connect(self._tick)
        self._active = False

    def load(self, settings: AppSettings) -> None:
        try:
            from ..services.recorder import list_input_devices
            devices = list_input_devices()
        except Exception as exc:
            devices = []
            self._error = f"Audio-System nicht verfügbar: {exc}"
        self._device.blockSignals(True)
        selected = 0
        for i, d in enumerate(devices):
            label = d["name"] + ("  (Standard)" if d.get("default") else "")
            self._device.addItem(label, d["index"])
            if settings.audio_input_device is not None and d["index"] == settings.audio_input_device:
                selected = i + 1
        self._device.setCurrentIndex(selected)
        self._device.blockSignals(False)

    def enter(self, settings: AppSettings) -> None:
        self._active = True
        self._restart_monitor()

    def leave(self) -> None:
        self._active = False
        self._stop_monitor()

    def _stop_monitor(self) -> None:
        self._timer.stop()
        if self._recorder is not None:
            try:
                self._recorder.discard()
            except Exception:
                pass
            self._recorder = None
        self._level.setValue(0)

    def _restart_monitor(self) -> None:
        if not self._active:
            return
        self._stop_monitor()
        self._peak = 0.0
        self._error = None
        try:
            from ..services.recorder import AudioRecorder
            self._recorder = AudioRecorder(device=self._device.currentData())
            self._recorder.start()
            self._error = self._recorder.error
        except Exception as exc:
            self._error = str(exc)
        if self._error:
            _status(self._status, "error", f"Das Mikrofon lässt sich nicht öffnen: {self._error}")
            self._recorder = None
            return
        _status(self._status, "hint", "Sprich jetzt ein paar Worte …")
        self._timer.start()

    def _tick(self) -> None:
        rec = self._recorder
        if rec is None:
            return
        level = rec.audio_level
        self._level.setValue(int(level * 100))
        with rec._lock:          # nur den Pegel anzeigen, keine Aufnahme sammeln
            rec._frames.clear()
        if level > self._peak:
            self._peak = level
            if self._peak > 0.15:
                _status(self._status, "success", "✓ BlitzVox hört dich.")
                _clear_wizard_error(self)

    def can_continue(self) -> tuple:
        if self._error:
            return False, "Bitte ein funktionierendes Mikrofon auswählen."
        return True, ""

    def store(self, settings: AppSettings) -> None:
        settings.audio_input_device = self._device.currentData()


# 5 ── Tastenkürzel ────────────────────────────────────────────────────────────

class HotkeyPage(WizardPage):
    # (Einstellungsfeld, Modus-Feld oder None, Name, Beschreibung)
    ROWS = [
        ("transcription",    "transcription_mode",    "🎙  BlitzVox",
         "Diktieren und einfügen, ohne KI"),
        ("text_improvement", "text_improvement_mode", "✨  BlitzVox+",
         "Text verbessern – für lange Texte und freies Sprechen"),
        ("dampf_ablassen",   "dampf_ablassen_mode",   "🔥  BlitzVox $%&!",
         "Frust in eine sachliche Nachricht umformulieren"),
        ("emoji_text",       "emoji_text_mode",       "😊  BlitzVox :)",
         "Text mit passenden Emojis"),
        ("correction",       None,                    "✏  Korrigieren",
         "Fenster öffnen, um den letzten Text zu korrigieren"),
    ]

    def __init__(self, parent=None):
        super().__init__(
            "Tastenkürzel",
            "Mit diesen Tastenkürzeln startest du jede Funktion aus jedem Programm heraus, "
            "ohne ein Fenster zu öffnen.",
            parent,
        )
        from .settings_window import HotkeyEdit  # erst hier importiert: vermeidet Zirkelbezug

        self._fields: dict = {}
        self._modes: dict = {}

        grid = QGridLayout()
        grid.setHorizontalSpacing(12)
        grid.setVerticalSpacing(4)
        grid.setColumnStretch(1, 3)
        grid.setColumnStretch(2, 2)
        for col, text in enumerate(("Funktion", "Tastenkürzel", "Modus")):
            head = QLabel(text)
            set_prop(head, "role", "label")
            grid.addWidget(head, 0, col)

        row = 1
        for key, mode_key, name, desc in self.ROWS:
            label = QLabel(name)
            grid.addWidget(label, row, 0)

            field = HotkeyEdit()
            field.setPlaceholderText("Feld anklicken, Tasten drücken")
            field.textChanged.connect(lambda *_: _clear_wizard_error(self))
            grid.addWidget(field, row, 1)
            self._fields[key] = field

            if mode_key:
                mode = QComboBox()
                mode.addItem("Halten", "hold")
                mode.addItem("Umschalten", "toggle")
                grid.addWidget(mode, row, 2)
                self._modes[mode_key] = mode
            row += 1

            grid.addWidget(_hint(desc), row, 0, 1, 3)
            row += 1
            grid.setRowMinimumHeight(row, 8)
            row += 1
        self.body.addLayout(grid)

        self.body.addWidget(_hint(
            "Halten: sprechen, solange die Tasten gedrückt sind. Umschalten: einmal "
            "drücken zum Starten, noch einmal zum Beenden. Zum Ändern in ein Feld klicken "
            "und die neue Kombination drücken. Mit der Rücktaste wird ein Feld geleert, "
            "die Funktion ist dann nur noch über das Tray-Symbol erreichbar. Das Kürzel "
            "für BlitzVox ist Pflicht, es wird beim Probediktat getestet."))

        if _session_is_wayland():
            warn = QLabel(
                "Du nutzt eine Wayland-Sitzung. Globale Tastenkürzel funktionieren dort nur "
                "eingeschränkt. Melde dich für volle Funktion mit einer X11-Sitzung an "
                "(unter Linux Mint: „Cinnamon“ statt „Cinnamon on Wayland“).")
            set_prop(warn, "role", "error")
            warn.setWordWrap(True)
            self.body.addWidget(warn)

    def load(self, settings: AppSettings) -> None:
        hk = settings.hotkeys
        for key, field in self._fields.items():
            field.setText(getattr(hk, key))
        for mode_key, combo in self._modes.items():
            combo.setCurrentIndex(0 if getattr(hk, mode_key) == "hold" else 1)

    @staticmethod
    def _valid(combo: str) -> bool:
        parts = [p.strip() for p in combo.split("+") if p.strip()]
        mods = {"ctrl", "alt", "shift", "meta", "cmd", "win", "super"}
        return any(p in mods for p in parts) and not all(p in mods for p in parts)

    def can_continue(self) -> tuple:
        names = {key: name.split("  ", 1)[-1] for key, _, name, _ in self.ROWS}
        if not self._fields["transcription"].text().strip():
            return False, "Bitte ein Tastenkürzel für „BlitzVox“ festlegen."
        seen: dict = {}
        for key, field in self._fields.items():
            combo = field.text().strip().lower()
            if not combo:
                continue
            if not self._valid(combo):
                return False, (f"Kürzel für „{names[key]}“: Bitte eine Kombination aus "
                               "Strg, Alt, Umschalt oder Win und einer weiteren Taste wählen.")
            if combo in seen:
                return False, (f"„{format_hotkey(combo)}“ ist doppelt vergeben: "
                               f"„{seen[combo]}“ und „{names[key]}“.")
            seen[combo] = names[key]
        return True, ""

    def store(self, settings: AppSettings) -> None:
        hk = settings.hotkeys
        for key, field in self._fields.items():
            setattr(hk, key, field.text().strip().lower())
        for mode_key, combo in self._modes.items():
            setattr(hk, mode_key, combo.currentData())
        hk.enabled = True


# 6 ── Probediktat ─────────────────────────────────────────────────────────────

class TestPage(WizardPage):
    _hk_start = pyqtSignal()
    _hk_stop = pyqtSignal()
    _phase = pyqtSignal(object, str)

    def __init__(self, parent=None):
        super().__init__(
            "Probediktat",
            "Jetzt prüfen wir alles zusammen: Tastenkürzel, Mikrofon und Spracherkennung.",
            parent,
        )
        self._settings: Optional[AppSettings] = None
        self._runner = None
        self._hotkeys = None
        self.succeeded = False

        self._instruction = QLabel()
        self._instruction.setWordWrap(True)
        set_prop(self._instruction, "role", "strong")
        self.body.addWidget(self._instruction)

        self._result = QTextEdit()
        self._result.setReadOnly(True)
        self._result.setPlaceholderText("Hier erscheint dein diktierter Text.")
        self._result.setFixedHeight(110)
        self.body.addWidget(self._result)

        row = QHBoxLayout()
        self._button = QPushButton("🎙  Aufnahme starten")
        self._button.clicked.connect(self._toggle_by_button)
        row.addWidget(self._button)
        row.addStretch(1)
        self.body.addLayout(row)

        self._status = QLabel()
        self._status.setWordWrap(True)
        self._status.hide()
        self.body.addWidget(self._status)

        self.body.addWidget(_hint(
            "Klappt das Tastenkürzel nicht, kannst du auch den Knopf benutzen. "
            "Im Alltag fügt BlitzVox den Text direkt dort ein, wo dein Cursor steht."))

        self._hk_start.connect(self._start)
        self._hk_stop.connect(self._stop)
        self._phase.connect(self._on_phase)

    # ── Seite betreten / verlassen ───────────────────────────────────────────

    def enter(self, settings: AppSettings) -> None:
        self._settings = settings
        hk = settings.hotkeys
        name = format_hotkey(hk.transcription)
        if hk.transcription_mode == "hold":
            text = f"Halte {name} gedrückt, sprich einen Satz und lass dann los."
        else:
            text = f"Drücke {name}, sprich einen Satz und drücke {name} noch einmal."
        self._instruction.setText(text)
        self._start_hotkeys()

    def leave(self) -> None:
        if self._hotkeys is not None:
            self._hotkeys.stop()
            self._hotkeys = None
        if self._runner is not None:
            self._runner.on_phase = None
            self._runner.cancel()
            self._runner = None
        self._button.setText("🎙  Aufnahme starten")
        self._button.setEnabled(True)

    def _start_hotkeys(self) -> None:
        try:
            from ..services.hotkeys import HotkeyService
            svc = HotkeyService()
        except Exception:
            svc = None
        if svc is None or not svc.is_available():
            _status(self._status, "hint",
                    "Tastenkürzel sind auf diesem System nicht verfügbar. "
                    "Bitte den Knopf benutzen.")
            return
        hk = self._settings.hotkeys
        if hk.transcription_mode == "hold":
            svc.register(hk.transcription, "hold",
                         on_press=self._hk_start.emit, on_release=self._hk_stop.emit)
        else:
            svc.register(hk.transcription, "toggle",
                         on_toggle=lambda on: self._hk_start.emit() if on
                         else self._hk_stop.emit())
        svc.start()
        self._hotkeys = svc

    # ── Aufnahme ─────────────────────────────────────────────────────────────

    def _recording(self) -> bool:
        return self._runner is not None and self._runner.is_recording

    def _toggle_by_button(self) -> None:
        if self._recording():
            self._stop()
        else:
            self._start()

    def _start(self) -> None:
        if self._recording() or self._settings is None or not self._button.isEnabled():
            return
        from ..workflow import WorkflowRunner, WorkflowKind
        runner = WorkflowRunner(
            WorkflowKind.TRANSCRIPTION, self._settings,
            on_phase=lambda phase, msg: self._phase.emit(phase, msg),
        )

        def run_async(audio_path, duration):
            threading.Thread(
                target=lambda: asyncio.run(runner._process(audio_path, duration)),
                daemon=True,
            ).start()

        runner._run_async = run_async  # wie im Workflow-Fenster: eigener Thread
        self._runner = runner
        runner.start_recording()

    def _stop(self) -> None:
        if self._recording():
            self._runner.stop_recording()

    def _on_phase(self, phase, message: str) -> None:
        from ..workflow import Phase
        if phase == Phase.RECORDING:
            self._button.setText("⏹  Aufnahme beenden")
            _status(self._status, "status", "🔴  Aufnahme läuft … sprich jetzt.")
        elif phase in (Phase.TRANSCRIBING, Phase.REWRITING):
            self._button.setText("🎙  Aufnahme starten")
            self._button.setEnabled(False)
            _status(self._status, "status", message)
        elif phase == Phase.DONE:
            self._button.setEnabled(True)
            self._result.setPlainText(message)
            self.succeeded = True
            _status(self._status, "success",
                    "✓ Alles funktioniert. Du kannst noch einmal testen oder weitergehen.")
        elif phase == Phase.ERROR:
            self._button.setText("🎙  Aufnahme starten")
            self._button.setEnabled(True)
            _status(self._status, "error",
                    f"{message}\nMit „Zurück“ kannst du die Einstellungen ändern.")


# 3 ── KI-Funktionen ───────────────────────────────────────────────────────────

class AiPage(WizardPage):
    def __init__(self, parent=None):
        super().__init__(
            "KI-Funktionen",
            "Für „Text verbessern“, „Frust umformulieren“ und „Emojis“ braucht "
            "BlitzVox ein Sprachmodell. Dieser Schritt ist optional.",
            parent,
        )
        self._token = 0
        self._or_token = 0
        self._ollama_ok = False
        self._stt_provider = None
        self._wanted_model = ""   # gespeichertes Modell, nach dem Laden der Liste wieder wählen

        self._enabled = QCheckBox("KI-Funktionen jetzt einrichten")
        self._enabled.setChecked(True)
        self._enabled.toggled.connect(self._update_mode)
        self.body.addWidget(self._enabled)

        self._box = QWidget()
        bl = QVBoxLayout(self._box)
        bl.setContentsMargins(0, 0, 0, 0)
        bl.setSpacing(10)

        self._overview = _overview_label()
        bl.addWidget(self._overview)

        form = QFormLayout()
        self._provider = QComboBox()
        for val in LLMProvider.ALL:
            if val == LLMProvider.CUSTOM:
                continue  # eigene Endpunkte nur in den Einstellungen
            self._provider.addItem(LLMProvider.DISPLAY[val], val)
        self._provider.currentIndexChanged.connect(self._on_provider_changed)
        form.addRow("Anbieter:", self._provider)
        self._model = QComboBox()
        self._model.setEditable(True)
        self._model.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        form.addRow("Modell:", self._model)
        bl.addLayout(form)
        self._model_status = _hint("")
        self._model_status.hide()
        bl.addWidget(self._model_status)

        self._same_key = _hint("Es wird derselbe Key wie bei der Spracherkennung verwendet.")
        self._same_key.setWordWrap(False)  # einzeilig, sonst falsche Höhe nach Ein-/Ausblenden
        bl.addWidget(self._same_key)

        self._key_box = KeyBox()
        self._key_box.key_saved.connect(lambda _: self._update_overview())
        bl.addWidget(self._key_box)
        bl.addWidget(_hint(
            "Auch hier kannst du mehrere Anbieter nacheinander einrichten. Genutzt wird "
            "der Anbieter, der beim Klick auf „Weiter“ ausgewählt ist."))

        # Ollama
        self._ollama_box = QWidget()
        ol = QVBoxLayout(self._ollama_box)
        ol.setContentsMargins(0, 0, 0, 0)
        orow = QHBoxLayout()
        self._ollama_url = QLineEdit()
        self._ollama_url.setPlaceholderText("http://localhost:11434")
        self._ollama_url.textChanged.connect(self._reset_ollama)
        orow.addWidget(QLabel("Adresse:"))
        orow.addWidget(self._ollama_url, 1)
        self._ollama_test = QPushButton("Verbindung testen")
        self._ollama_test.clicked.connect(self._test_ollama)
        orow.addWidget(self._ollama_test)
        ol.addLayout(orow)
        self._ollama_status = QLabel()
        self._ollama_status.setWordWrap(True)
        self._ollama_status.hide()
        ol.addWidget(self._ollama_status)
        ol.addWidget(_hint(
            "Ollama muss auf deinem Rechner installiert sein und laufen. Das Modell "
            "lädst du vorher mit „ollama pull <name>“ herunter."))
        bl.addWidget(self._ollama_box)

        self.body.addWidget(self._box)
        self._skip_hint = _hint(
            "Ohne Sprachmodell funktioniert die normale Transkription weiterhin. "
            "Die KI-Funktionen kannst du später in den Einstellungen einrichten.")
        self.body.addWidget(self._skip_hint)

    def _provider_value(self) -> str:
        return self._provider.currentData()

    def _on_provider_changed(self) -> None:
        p = self._provider_value()
        self._model.clear()
        self._model_status.hide()
        if p == LLMProvider.OPENROUTER:
            # Liste live von OpenRouter laden (wie im Einstellungsfenster)
            self._model.lineEdit().setPlaceholderText("Modell-ID, z. B. von openrouter.ai/models")
            self._load_openrouter_models()
        elif p == LLMProvider.OLLAMA:
            # Nur Modelle zeigen, die in Ollama wirklich installiert sind
            self._model.lineEdit().setPlaceholderText("Erst „Verbindung testen“")
            self._test_ollama()
        else:
            self._model.lineEdit().setPlaceholderText("")
            self._model.addItems(LLM_MODEL_PRESETS.get(p, []))
        self._update_mode()

    def _fill_models(self, names: list) -> None:
        self._model.clear()
        self._model.addItems(names)
        if self._wanted_model and self._wanted_model in names:
            self._model.setCurrentIndex(names.index(self._wanted_model))
        elif self._wanted_model and self._provider_value() == LLMProvider.OPENROUTER:
            self._model.setEditText(self._wanted_model)

    def _load_openrouter_models(self) -> None:
        self._or_token += 1
        token = self._or_token
        _status(self._model_status, "hint", "Modelle werden von OpenRouter geladen …")
        from ..services.settings import fetch_openrouter_models

        def done(ids):
            if token != self._or_token or self._provider_value() != LLMProvider.OPENROUTER:
                return
            ids = ids if isinstance(ids, list) else []
            self._fill_models(ids)
            if ids:
                _status(self._model_status, "hint",
                        f"{len(ids)} Modelle geladen. Auswählen oder eine Modell-ID eintippen.")
            else:
                self._model.setEditText(self._wanted_model)
                _status(self._model_status, "hint",
                        "Modellliste nicht erreichbar (offline?). Modell-ID bitte von Hand eintragen.")

        _run_in_thread(fetch_openrouter_models, done)

    def _key_shared(self, key_name: Optional[str]) -> bool:
        """True, wenn das Sprachmodell denselben, schon gespeicherten Key wie die STT nutzt."""
        return (key_name is not None
                and key_name == _STT_KEY_REQUIRED.get(self._stt_provider)
                and get_api_key(key_name) is not None)

    def _update_overview(self) -> None:
        names = [LLMProvider.DISPLAY[p].split(" (")[0]
                 for p, key in _LLM_KEY_REQUIRED.items() if get_api_key(key)]
        if self._ollama_ok:
            names.append("Lokal – Ollama")
        _set_overview(self._overview, names)

    def _update_mode(self) -> None:
        self._update_overview()
        on = self._enabled.isChecked()
        self._box.setVisible(on)
        self._skip_hint.setVisible(not on)
        p = self._provider_value()
        key_name = _LLM_KEY_REQUIRED.get(p)
        shared = self._key_shared(key_name)
        self._same_key.setVisible(shared)
        self._key_box.setVisible(key_name is not None and not shared)
        if key_name:
            self._key_box.set_key_name(key_name)
        self._ollama_box.setVisible(p == LLMProvider.OLLAMA)

    def _reset_ollama(self) -> None:
        self._ollama_ok = False
        _status(self._ollama_status, "hint", "")
        if self._provider_value() == LLMProvider.OLLAMA:
            self._model.clear()

    def _test_ollama(self) -> None:
        url = self._ollama_url.text().strip() or "http://localhost:11434"
        self._token += 1
        token = self._token
        self._ollama_test.setEnabled(False)
        _status(self._ollama_status, "hint", "Verbindung wird geprüft …")

        def done(res):
            if token != self._token:
                return
            self._ollama_test.setEnabled(True)
            status, msg = res[0], res[1]
            names = res[2] if len(res) > 2 else []
            self._ollama_ok = status == "ok"
            _status(self._ollama_status, "success" if self._ollama_ok else "error", msg)
            if self._provider_value() == LLMProvider.OLLAMA:
                self._fill_models(names)
            if self._ollama_ok:
                _clear_wizard_error(self)
            self._update_overview()

        _run_in_thread(lambda: check_ollama(url), done)

    def load(self, settings: AppSettings) -> None:
        cfg = settings.text_improvement.model
        idx = self._provider.findData(cfg.llm_provider)
        model = (settings.openrouter_llm_model if cfg.llm_provider == LLMProvider.OPENROUTER
                 else cfg.llm_model)
        self._wanted_model = model or ""
        self._ollama_url.blockSignals(True)
        self._ollama_url.setText(settings.ollama_base_url)
        self._ollama_url.blockSignals(False)
        self._provider.blockSignals(True)
        self._provider.setCurrentIndex(max(idx, 0))
        self._provider.blockSignals(False)
        self._on_provider_changed()
        if model and self._provider_value() not in (LLMProvider.OPENROUTER, LLMProvider.OLLAMA):
            self._model.setEditText(model)

    def enter(self, settings: AppSettings) -> None:
        # Der STT-Dienst kann sich auf Seite 2 geändert haben -> Key-Anzeige anpassen
        self._stt_provider = settings.transcription_model.stt_provider
        self._update_mode()

    def can_continue(self) -> tuple:
        if not self._enabled.isChecked():
            return True, ""
        p = self._provider_value()
        if p == LLMProvider.OLLAMA and not self._ollama_ok:
            return False, ("Ollama läuft nicht oder hat noch kein Modell. Bitte prüfen und "
                           "„Verbindung testen“ klicken.")
        if not self._model.currentText().strip():
            return False, "Bitte ein Modell auswählen oder eintragen."
        if p in _LLM_KEY_REQUIRED and not self._key_shared(_LLM_KEY_REQUIRED[p]):
            return self._key_box.commit()
        return True, ""

    def store(self, settings: AppSettings) -> None:
        if not self._enabled.isChecked():
            return
        p = self._provider_value()
        model = self._model.currentText().strip()
        for cfg in _llm_models(settings):
            cfg.llm_provider = p
            cfg.llm_model = model
        if p == LLMProvider.OPENROUTER:
            settings.openrouter_llm_model = model
        elif p == LLMProvider.OLLAMA:
            settings.ollama_base_url = self._ollama_url.text().strip() or "http://localhost:11434"
            settings.ollama_llm_model = model


# 7 ── Abschluss ───────────────────────────────────────────────────────────────

class FinishPage(WizardPage):
    def __init__(self, parent=None):
        super().__init__(
            "Fast fertig",
            "Noch zwei Grundeinstellungen. Alles Weitere findest du jederzeit "
            "in den Einstellungen.",
            parent,
        )
        self._auto_paste = QCheckBox("Text automatisch dort einfügen, wo der Cursor steht")
        self.body.addWidget(self._auto_paste)
        self.body.addWidget(_hint(
            "Ohne diese Option landet der Text nur in der Zwischenablage und du fügst ihn "
            "selbst mit Strg+V ein."))

        self._paste_warn = QLabel()
        set_prop(self._paste_warn, "role", "error")
        self._paste_warn.setWordWrap(True)
        self._paste_warn.hide()
        self.body.addWidget(self._paste_warn)

        self._autostart = QCheckBox(
            "BlitzVox mit Windows starten" if platform.system() == "Windows"
            else "BlitzVox beim Anmelden starten")
        self.body.addWidget(self._autostart)

        self.body.addSpacing(8)
        head = QLabel("Zusammenfassung")
        set_prop(head, "role", "label")
        self.body.addWidget(head)
        self._summary = QLabel()
        self._summary.setWordWrap(True)
        self._summary.setTextFormat(Qt.TextFormat.RichText)
        self.body.addWidget(self._summary)

    def load(self, settings: AppSettings) -> None:
        self._auto_paste.setChecked(settings.auto_paste)
        self._autostart.setChecked(settings.launch_at_login)
        if (platform.system() == "Linux" and not _session_is_wayland()
                and shutil.which("xdotool") is None):
            self._paste_warn.setText(
                "Für das automatische Einfügen fehlt das Programm xdotool. "
                "Installieren mit: sudo apt install xdotool")
            self._paste_warn.show()

    def enter(self, settings: AppSettings) -> None:
        lang_code = settings.transcription_model.language
        lang = next((n for n, c in LANGUAGES if c == lang_code), lang_code or "—")
        stt = STTProvider.DISPLAY.get(settings.transcription_model.stt_provider, "—")
        hk = settings.hotkeys

        def key_text(key: str, mode_key: Optional[str]) -> str:
            combo = getattr(hk, key)
            if not combo:
                return "kein Kürzel"
            if mode_key is None:
                return format_hotkey(combo)
            mode = "halten" if getattr(hk, mode_key) == "hold" else "umschalten"
            return f"{format_hotkey(combo)} ({mode})"

        llm_cfg = settings.text_improvement.model
        llm = LLMProvider.DISPLAY.get(llm_cfg.llm_provider, "—")
        if llm_cfg.llm_model:
            llm += f" – {llm_cfg.llm_model}"
        rows = [
            ("Sprache", lang),
            ("Spracherkennung", stt),
            ("KI-Funktionen", llm),
        ]
        hotkeys = [(name.split("  ", 1)[-1], key_text(key, mode_key))
                   for key, mode_key, name, _ in HotkeyPage.ROWS]
        html = "<br>".join(f"<b>{k}:</b> {v}" for k, v in rows)
        html += "<br><br><b>Tastenkürzel</b><br>" + "<br>".join(
            f"{k}: {v}" for k, v in hotkeys)
        self._summary.setText(html)

    def store(self, settings: AppSettings) -> None:
        settings.auto_paste = self._auto_paste.isChecked()
        settings.launch_at_login = self._autostart.isChecked()


# ── Assistent ────────────────────────────────────────────────────────────────

class SetupWizard(QDialog):
    completed = pyqtSignal(object)   # gibt die geänderten AppSettings zurück

    def __init__(self, settings: AppSettings, parent=None):
        super().__init__(parent)
        self._settings = copy.deepcopy(settings)

        self.setWindowTitle(f"BlitzVox – Einrichtung (v{__version__})")
        self.setWindowFlags(Qt.WindowType.Window | Qt.WindowType.WindowCloseButtonHint)
        self.setMinimumSize(660, 580)

        self._pages: list = [
            WelcomePage(), SpeechPage(), AiPage(), MicrophonePage(),
            HotkeyPage(), TestPage(), FinishPage(),
        ]

        self._stack = QStackedWidget()
        for page in self._pages:
            page.load(self._settings)
            self._stack.addWidget(page)

        # Fußzeile: Schrittanzeige, Fehlermeldung, Knöpfe
        self._step_label = QLabel()
        set_prop(self._step_label, "role", "hint")

        self._error_label = QLabel()
        set_prop(self._error_label, "role", "error")
        self._error_label.setWordWrap(True)
        self._error_label.hide()

        self._cancel_btn = QPushButton("Abbrechen")
        set_prop(self._cancel_btn, "variant", "text")
        self._back_btn = QPushButton("Zurück")
        self._next_btn = QPushButton("Weiter")
        set_prop(self._next_btn, "variant", "primary")

        self._cancel_btn.clicked.connect(self.reject)
        self._back_btn.clicked.connect(self._go_back)
        self._next_btn.clicked.connect(self._go_next)

        buttons = QHBoxLayout()
        buttons.addWidget(self._step_label)
        buttons.addStretch(1)
        buttons.addWidget(self._cancel_btn)
        buttons.addWidget(self._back_btn)
        buttons.addWidget(self._next_btn)

        line = QFrame()
        line.setFrameShape(QFrame.Shape.HLine)

        footer = QVBoxLayout()
        footer.setContentsMargins(32, 8, 32, 20)
        footer.addWidget(self._error_label)
        footer.addLayout(buttons)

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        layout.addWidget(self._stack, 1)
        layout.addWidget(line)
        layout.addLayout(footer)

        self._show_page(0)

    # ── Navigation ───────────────────────────────────────────────────────────

    def _current(self) -> WizardPage:
        return self._pages[self._stack.currentIndex()]

    def _show_page(self, idx: int) -> None:
        self._stack.setCurrentIndex(idx)
        last = idx == len(self._pages) - 1
        self._step_label.setText(f"Schritt {idx + 1} von {len(self._pages)}")
        self._back_btn.setEnabled(idx > 0)
        self._next_btn.setText("Fertig" if last else "Weiter")
        self._error_label.hide()
        self._current().enter(self._settings)

    def _go_back(self) -> None:
        idx = self._stack.currentIndex()
        if idx > 0:
            page = self._current()
            page.leave()
            page.store(self._settings)
            self._show_page(idx - 1)

    def _go_next(self) -> None:
        page = self._current()
        ok, message = page.can_continue()
        if not ok:
            self._error_label.setText(message)
            self._error_label.show()
            return
        page.leave()
        page.store(self._settings)

        idx = self._stack.currentIndex()
        if idx < len(self._pages) - 1:
            self._show_page(idx + 1)
        else:
            self._finish()

    def clear_error(self) -> None:
        self._error_label.hide()

    def _finish(self) -> None:
        self._settings.has_seen_onboarding = True
        self.completed.emit(self._settings)
        self.accept()

    # ── Abbruch ──────────────────────────────────────────────────────────────

    def reject(self) -> None:
        """Abbrechen oder Fenster schließen: Eingaben verwerfen. API-Keys, die
        beim Weiterklicken schon gespeichert wurden, bleiben erhalten."""
        self._current().leave()
        super().reject()


def run_setup_wizard(settings: AppSettings, parent=None) -> Optional[AppSettings]:
    """Zeigt den Assistenten modal. Liefert die neuen Einstellungen bei „Fertig“,
    sonst None."""
    wizard = SetupWizard(settings, parent)
    result: list = []
    wizard.completed.connect(result.append)
    wizard.exec()
    return result[0] if result else None
