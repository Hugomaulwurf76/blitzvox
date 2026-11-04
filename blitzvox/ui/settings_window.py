"""
Settings window — tabbed UI.
Tabs: API-Zugänge | Workflows | Hotkeys | Korrekturen | Erweitert
"""

import platform
import threading
from PyQt6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel, QLineEdit,
    QPushButton, QTabWidget, QFormLayout, QComboBox, QCheckBox,
    QTextEdit, QGroupBox, QScrollArea, QSizePolicy, QFrame,
    QSpinBox, QMessageBox, QTableWidget, QTableWidgetItem, QHeaderView,
    QGridLayout,
    QAbstractItemView,
)
from PyQt6.QtCore import Qt, QObject, pyqtSignal
from PyQt6.QtGui import QFont, QKeySequence

from .theme import set_prop

from .. import __version__
from ..services.settings import (
    AppSettings, save_settings,
    STTProvider, LLMProvider, WorkflowModelConfig, LLM_MODEL_PRESETS,
    get_api_key, set_api_key, masked,
    KEY_OPENAI, KEY_GROQ, KEY_ANTHROPIC, KEY_ASSEMBLYAI, KEY_CUSTOM_STT, KEY_CUSTOM_LLM,
    KEY_OPENROUTER, fetch_openrouter_models, fetch_openrouter_transcription_models,
)


class NoWheelComboBox(QComboBox):
    """Auswahlliste ohne Mausrad-Wechsel: Das Rad-Ereignis wird ignoriert und
    an den Elternbereich (Scroll-Bereich) weitergereicht, damit die Seite
    scrollt, statt versehentlich den Eintrag zu wechseln. Im aufgeklappten
    Popup funktioniert das Mausrad weiterhin normal."""

    def wheelEvent(self, event):
        event.ignore()


class HotkeyEdit(QLineEdit):
    """Nimmt die gedrückte Tastenkombination auf, z. B. 'ctrl+shift+g'."""

    _MODIFIER_KEYS = {
        Qt.Key.Key_Control, Qt.Key.Key_Shift, Qt.Key.Key_Alt,
        Qt.Key.Key_Meta, Qt.Key.Key_AltGr,
    }

    def keyPressEvent(self, event):
        key = event.key()
        if key in self._MODIFIER_KEYS:
            return  # nur eine Modifier-Taste: noch nichts übernehmen
        if key in (Qt.Key.Key_Backspace, Qt.Key.Key_Delete) \
                and event.modifiers() == Qt.KeyboardModifier.NoModifier:
            self.clear()
            return
        combo = QKeySequence(event.keyCombination()).toString(
            QKeySequence.SequenceFormat.PortableText)
        if combo:
            self.setText(combo.lower())


class _ModelLoader(QObject):
    loaded = pyqtSignal(list)
    transcription_loaded = pyqtSignal(list)


class SettingsWindow(QWidget):
    settings_saved = pyqtSignal(object)  # emits AppSettings

    def __init__(self, settings: AppSettings, parent=None):
        super().__init__(parent)
        self.settings = settings
        self._or_loader = _ModelLoader()
        self._or_loader.loaded.connect(self._on_openrouter_models)
        self._or_loading = False
        self._llm_model_combos = []  # (Modell-Combo, zugehörige Anbieter-Combo)
        self._stt_model_combos = []  # (STT-Modell-Combo, zugehörige Anbieter-Combo)
        self._or_stt_models = []
        self._or_loader.transcription_loaded.connect(self._on_openrouter_transcription_models)
        self._setup_ui()
        self.setWindowTitle(f"BlitzVox – Einstellungen (v{__version__})")
        self.setWindowFlags(Qt.WindowType.Window | Qt.WindowType.WindowCloseButtonHint)
        self.setMinimumWidth(620)
        self.setMinimumHeight(720)

    def _setup_ui(self) -> None:
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)

        tabs = QTabWidget()
        tabs.addTab(self._tab_api(),       "🔑  API-Zugänge")
        tabs.addTab(self._tab_workflows(), "⚙  Workflows")
        tabs.addTab(self._tab_hotkeys(),   "⌨  Hotkeys")
        tabs.addTab(self._tab_corrections(), "📝  Korrekturen")
        tabs.addTab(self._tab_advanced(),  "🔧  Erweitert")
        layout.addWidget(tabs)

        # Save / Cancel
        btn_row = QHBoxLayout()
        btn_row.setContentsMargins(20, 12, 20, 18)
        btn_row.setSpacing(10)
        btn_row.addStretch()

        cancel = QPushButton("Abbrechen")
        cancel.clicked.connect(self.close)
        btn_row.addWidget(cancel)

        save = QPushButton("Speichern")
        save.setDefault(True)
        set_prop(save, "variant", "primary")
        save.clicked.connect(self._save)
        btn_row.addWidget(save)

        layout.addLayout(btn_row)

    # ── Tab: API-Zugänge ─────────────────────────────────────────────────────

    def _tab_api(self) -> QWidget:
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        w = QWidget()
        layout = QVBoxLayout(w)
        layout.setContentsMargins(20, 14, 20, 20)
        layout.setSpacing(14)

        def _key_row(label: str, key_name: str, placeholder: str = "sk-…") -> QWidget:
            grp = QGroupBox(label)
            g = QVBoxLayout(grp)
            g.setSpacing(8)

            current = get_api_key(key_name)
            display = QLabel(f"Gespeichert: {masked(current)}" if current else "Nicht gesetzt")
            set_prop(display, "role", "hint")
            g.addWidget(display)

            row = QHBoxLayout()
            row.setSpacing(8)

            field = QLineEdit()
            field.setPlaceholderText(placeholder)
            field.setEchoMode(QLineEdit.EchoMode.Password)
            field.setObjectName(f"key_{key_name}")
            row.addWidget(field, 1)

            show_btn = QPushButton("👁")
            set_prop(show_btn, "variant", "icon")
            show_btn.setCheckable(True)
            show_btn.setToolTip("Schlüssel anzeigen / verbergen")
            show_btn.toggled.connect(
                lambda on, f=field: f.setEchoMode(
                    QLineEdit.EchoMode.Normal if on else QLineEdit.EchoMode.Password
                )
            )
            row.addWidget(show_btn)

            paste_btn = QPushButton("Einfügen")
            paste_btn.setMinimumWidth(80)
            paste_btn.clicked.connect(lambda: self._paste_key(field))
            row.addWidget(paste_btn)

            clear_btn = QPushButton("Löschen")
            clear_btn.setMinimumWidth(80)
            clear_btn.clicked.connect(lambda: self._clear_key(key_name, display, field))
            row.addWidget(clear_btn)

            g.addLayout(row)
            return grp

        groq_hint = QLabel("👉 Kostenlos starten: https://console.groq.com  (STT + LLM gratis)")
        set_prop(groq_hint, "role", "lead")
        layout.addWidget(groq_hint)

        layout.addWidget(_key_row("Groq API Key (kostenlos, Standard)", KEY_GROQ, "gsk_…"))
        layout.addWidget(_key_row("Claude API Key (Anthropic)", KEY_ANTHROPIC, "sk-ant-…"))
        layout.addWidget(_key_row("OpenAI API Key", KEY_OPENAI))
        layout.addWidget(_key_row("AssemblyAI API Key", KEY_ASSEMBLYAI, "…"))

        # Ollama URL
        ollama_grp = QGroupBox("Ollama (lokales LLM)")
        ol = QFormLayout(ollama_grp)
        self._ollama_url = QLineEdit(self.settings.ollama_base_url)
        self._ollama_url.setPlaceholderText("http://localhost:11434")
        self._ollama_model = QLineEdit(self.settings.ollama_llm_model)
        self._ollama_model.setPlaceholderText("llama3.2")
        ol.addRow("Basis-URL:", self._ollama_url)
        ol.addRow("Modell:", self._ollama_model)
        layout.addWidget(ollama_grp)

        # Custom endpoints
        custom_grp = QGroupBox("Eigene Endpunkte (OpenAI-kompatibel)")
        cu = QFormLayout(custom_grp)
        self._custom_stt_url = QLineEdit(self.settings.custom_stt_url)
        self._custom_stt_url.setPlaceholderText("https://…/v1/audio/transcriptions")
        self._custom_llm_url = QLineEdit(self.settings.custom_llm_url)
        self._custom_llm_url.setPlaceholderText("https://…/v1/chat/completions")
        cu.addRow("STT-Endpunkt:", self._custom_stt_url)
        cu.addRow("LLM-Endpunkt:", self._custom_llm_url)

        custom_stt_key = get_api_key(KEY_CUSTOM_STT)
        self._custom_stt_key = QLineEdit()
        self._custom_stt_key.setPlaceholderText(
            f"Gespeichert: {masked(custom_stt_key)}" if custom_stt_key else "Nicht gesetzt"
        )
        self._custom_stt_key.setEchoMode(QLineEdit.EchoMode.Password)
        self._custom_stt_key.setObjectName(f"key_{KEY_CUSTOM_STT}")
        cu.addRow("STT-Key (optional):", self._custom_stt_key)

        custom_llm_key = get_api_key(KEY_CUSTOM_LLM)
        self._custom_llm_key = QLineEdit()
        self._custom_llm_key.setPlaceholderText(
            f"Gespeichert: {masked(custom_llm_key)}" if custom_llm_key else "Nicht gesetzt"
        )
        self._custom_llm_key.setEchoMode(QLineEdit.EchoMode.Password)
        self._custom_llm_key.setObjectName(f"key_{KEY_CUSTOM_LLM}")
        cu.addRow("LLM-Key (optional):", self._custom_llm_key)

        # OpenRouter: Key + frei wählbares Modell aus der Live-Liste
        layout.addWidget(_key_row("OpenRouter API Key", KEY_OPENROUTER, "sk-or-v1-…"))

        or_model_grp = QGroupBox("OpenRouter – Modell")
        om = QFormLayout(or_model_grp)
        self._or_model = NoWheelComboBox()
        self._or_model.setEditable(True)
        self._or_model.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        if self.settings.openrouter_llm_model:
            self._or_model.addItem(self.settings.openrouter_llm_model)
        self._or_status = QLabel("Modelle werden geladen …")
        set_prop(self._or_status, "role", "hint")
        self._or_model.currentTextChanged.connect(self._refresh_openrouter_workflow_combos)
        om.addRow("Modell:", self._or_model)
        om.addRow("", self._or_status)
        layout.addWidget(or_model_grp)

        layout.addWidget(custom_grp)

        layout.addStretch()
        scroll.setWidget(w)
        return scroll

    def _paste_key(self, field: QLineEdit) -> None:
        from PyQt6.QtWidgets import QApplication
        text = QApplication.clipboard().text().strip()
        if text:
            field.setText(text)

    def _clear_key(self, key_name: str, display: QLabel, field: QLineEdit) -> None:
        from ..services.settings import delete_api_key
        delete_api_key(key_name)
        display.setText("Nicht gesetzt")
        field.clear()

    # ── Tab: Workflows ───────────────────────────────────────────────────────

    def _tab_workflows(self) -> QWidget:
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        w = QWidget()
        layout = QVBoxLayout(w)
        layout.setContentsMargins(20, 14, 20, 20)
        layout.setSpacing(16)

        # ─ Transkription ─
        tr_grp = QGroupBox("🎙  BlitzVox — Transkription")
        tr = QFormLayout(tr_grp)
        self._tr_stt_provider = self._stt_combo(self.settings.transcription_model.stt_provider)
        self._tr_stt_model    = self._stt_model_combo(self._tr_stt_provider, self.settings.transcription_model.stt_model)
        self._tr_language     = QLineEdit(self.settings.transcription_model.language)
        tr.addRow("STT-Anbieter:", self._tr_stt_provider)
        tr.addRow("STT-Modell:", self._tr_stt_model)
        tr.addRow("Sprache (BCP-47):", self._tr_language)
        layout.addWidget(tr_grp)

        # ─ BlitzVox+ ─
        ti_grp = QGroupBox("✨  BlitzVox+ — Text verbessern")
        ti = QFormLayout(ti_grp)
        self._ti_stt_provider = self._stt_combo(self.settings.text_improvement.model.stt_provider)
        self._ti_stt_model    = self._stt_model_combo(self._ti_stt_provider, self.settings.text_improvement.model.stt_model)
        self._ti_llm_provider = self._llm_combo(self.settings.text_improvement.model.llm_provider)
        self._ti_llm_model    = self._llm_model_combo(
            self._ti_llm_provider, self.settings.text_improvement.model.llm_model
        )
        self._ti_tone         = NoWheelComboBox()
        for val, lbl in [("formal","Formell"),("neutral","Neutral"),("casual","Locker")]:
            self._ti_tone.addItem(lbl, val)
        self._ti_tone.setCurrentIndex(
            ["formal","neutral","casual"].index(self.settings.text_improvement.tone)
        )
        self._ti_context = QLineEdit(self.settings.text_improvement.context)
        self._ti_prompt  = QTextEdit(self.settings.text_improvement.system_prompt)
        self._ti_prompt.setFixedHeight(80)
        self._ti_prompt.setPlaceholderText("Leer = automatischer Standardprompt")
        ti.addRow("STT-Anbieter:", self._ti_stt_provider)
        ti.addRow("STT-Modell:", self._ti_stt_model)
        ti.addRow("LLM-Anbieter:", self._ti_llm_provider)
        ti.addRow("LLM-Modell:", self._ti_llm_model)
        ti.addRow("Ton:", self._ti_tone)
        ti.addRow("Kontext:", self._ti_context)
        ti.addRow("Eigener Prompt:", self._ti_prompt)
        layout.addWidget(ti_grp)

        # ─ Dampf ablassen ─
        da_grp = QGroupBox("🔥  BlitzVox $%&! — Frust umformulieren")
        da = QFormLayout(da_grp)
        self._da_stt_provider = self._stt_combo(self.settings.dampf_ablassen.model.stt_provider)
        self._da_stt_model    = self._stt_model_combo(self._da_stt_provider, self.settings.dampf_ablassen.model.stt_model)
        self._da_llm_provider = self._llm_combo(self.settings.dampf_ablassen.model.llm_provider)
        self._da_llm_model    = self._llm_model_combo(
            self._da_llm_provider, self.settings.dampf_ablassen.model.llm_model
        )
        self._da_prompt       = QTextEdit(self.settings.dampf_ablassen.system_prompt)
        self._da_prompt.setFixedHeight(100)
        da.addRow("STT-Anbieter:", self._da_stt_provider)
        da.addRow("STT-Modell:", self._da_stt_model)
        da.addRow("LLM-Anbieter:", self._da_llm_provider)
        da.addRow("LLM-Modell:", self._da_llm_model)
        da.addRow("System-Prompt:", self._da_prompt)
        layout.addWidget(da_grp)

        # ─ Emoji ─
        em_grp = QGroupBox("😊  BlitzVox :) — Emojis")
        em = QFormLayout(em_grp)
        self._em_stt_provider = self._stt_combo(self.settings.emoji.model.stt_provider)
        self._em_stt_model    = self._stt_model_combo(self._em_stt_provider, self.settings.emoji.model.stt_model)
        self._em_llm_provider = self._llm_combo(self.settings.emoji.model.llm_provider)
        self._em_llm_model    = self._llm_model_combo(
            self._em_llm_provider, self.settings.emoji.model.llm_model
        )
        self._em_density      = NoWheelComboBox()
        for val, lbl in [("wenig","Wenig"),("mittel","Mittel"),("viel","Viel")]:
            self._em_density.addItem(lbl, val)
        self._em_density.setCurrentIndex(
            ["wenig","mittel","viel"].index(self.settings.emoji.density)
        )
        em.addRow("STT-Anbieter:", self._em_stt_provider)
        em.addRow("STT-Modell:", self._em_stt_model)
        em.addRow("LLM-Anbieter:", self._em_llm_provider)
        em.addRow("LLM-Modell:", self._em_llm_model)
        em.addRow("Emoji-Dichte:", self._em_density)
        layout.addWidget(em_grp)

        layout.addStretch()
        scroll.setWidget(w)
        return scroll

    def _stt_model_combo(self, provider_combo: QComboBox, current_model: str) -> QComboBox:
        """Editierbares STT-Modellfeld. Bei OpenRouter kommt die Liste von der API."""
        cb = NoWheelComboBox()
        cb.setEditable(True)
        cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        self._stt_model_combos.append((cb, provider_combo))
        self._fill_stt_models(cb, provider_combo.currentData(), current_model)
        # currentIndexChanged feuert nur bei echtem Anbieterwechsel, nicht beim
        # bloßen Aufklappen oder erneuten Wählen desselben Eintrags.
        # Bei Wechsel wird das Modellfeld geleert.
        provider_combo.currentIndexChanged.connect(
            lambda: self._fill_stt_models(cb, provider_combo.currentData(), "")
        )
        return cb

    def _fill_stt_models(self, cb: QComboBox, provider: str, keep_text: str) -> None:
        cb.blockSignals(True)
        cb.clear()
        if provider == STTProvider.OPENROUTER:
            cb.addItems(self._or_stt_models)
        elif provider == STTProvider.OPENAI:
            cb.addItem("whisper-1")  # Standardmodell aus services/stt.py
        elif provider == STTProvider.GROQ:
            cb.addItem("whisper-large-v3-turbo")  # Standardmodell aus services/stt.py
        cb.setEditText(keep_text)
        cb.blockSignals(False)

    def _on_openrouter_transcription_models(self, ids: list) -> None:
        self._or_stt_models = ids
        for cb, provider_combo in self._stt_model_combos:
            if provider_combo.currentData() == STTProvider.OPENROUTER:
                self._fill_stt_models(cb, STTProvider.OPENROUTER, cb.currentText())

    def _stt_combo(self, current: str) -> QComboBox:
        cb = NoWheelComboBox()
        for val in STTProvider.ALL:
            cb.addItem(STTProvider.DISPLAY[val], val)
        idx = STTProvider.ALL.index(current) if current in STTProvider.ALL else 0
        cb.setCurrentIndex(idx)
        return cb

    def _llm_combo(self, current: str) -> QComboBox:
        cb = NoWheelComboBox()
        for val in LLMProvider.ALL:
            cb.addItem(LLMProvider.DISPLAY[val], val)
        idx = LLMProvider.ALL.index(current) if current in LLMProvider.ALL else 0
        cb.setCurrentIndex(idx)
        return cb

    def _llm_model_combo(self, provider_combo: QComboBox, current_model: str) -> QComboBox:
        """Editierbare Auswahlliste fuer das LLM-Modell, Vorschlaege richten sich
        nach dem aktuell gewaehlten LLM-Anbieter. Eigene Modellnamen bleiben
        weiterhin moeglich (Feld ist editierbar)."""
        cb = NoWheelComboBox()
        cb.setEditable(True)
        cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        self._llm_model_combos.append((cb, provider_combo))
        self._fill_llm_models(cb, provider_combo.currentData(), current_model)
        # Nur bei echtem Anbieterwechsel feuern; dann Modellfeld leeren.
        provider_combo.currentIndexChanged.connect(
            lambda: self._fill_llm_models(cb, provider_combo.currentData(), "")
        )
        return cb

    def _fill_llm_models(self, cb: QComboBox, provider: str, keep_text: str) -> None:
        cb.blockSignals(True)
        cb.clear()
        if provider == LLMProvider.OPENROUTER:
            # Modell kommt aus dem Block "OpenRouter – Modell" (API-Zugänge)
            cb.addItem(self._or_model.currentText().strip())
            cb.setEnabled(False)
            cb.setToolTip("Wird im Tab API-Zugänge unter 'OpenRouter – Modell' gewählt.")
        else:
            cb.setEnabled(True)
            cb.setToolTip("")
            cb.addItems(LLM_MODEL_PRESETS.get(provider, []))
            cb.setEditText(keep_text)
        cb.blockSignals(False)

    def _refresh_openrouter_workflow_combos(self, *_) -> None:
        for cb, provider_combo in self._llm_model_combos:
            if provider_combo.currentData() == LLMProvider.OPENROUTER:
                self._fill_llm_models(cb, LLMProvider.OPENROUTER, "")

    # ── Tab: Hotkeys ─────────────────────────────────────────────────────────

    def _tab_hotkeys(self) -> QWidget:
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)

        w = QWidget()
        layout = QVBoxLayout(w)
        layout.setContentsMargins(20, 14, 20, 20)
        layout.setSpacing(14)

        intro = QLabel(
            "Mit Tastenkürzeln startest du BlitzVox aus jedem Programm heraus, "
            "ohne ein Fenster zu öffnen. Mit Auto-Paste (Tab Erweitert) landet der "
            "Text dort, wo der Cursor steht, sonst in der Zwischenablage."
        )
        set_prop(intro, "role", "lead")
        intro.setWordWrap(True)
        layout.addWidget(intro)

        self._hk_enabled = QCheckBox("Globale Hotkeys aktivieren")
        self._hk_enabled.setChecked(self.settings.hotkeys.enabled)
        layout.addWidget(self._hk_enabled)

        hk_note = QLabel(
            "Ist der Haken gesetzt, funktionieren die Tastenkürzel unten in jedem "
            "Programm. Ohne Haken reagiert BlitzVox auf kein Tastenkürzel – auch "
            "nicht auf „Korrigieren“. Die Workflows erreichst du dann nur über das "
            "BlitzVox-Symbol im Tray. Die Änderung gilt sofort nach „Speichern“."
        )
        set_prop(hk_note, "role", "hint")
        hk_note.setWordWrap(True)
        layout.addWidget(hk_note)

        def _mode_combo(current: str) -> QComboBox:
            cb = NoWheelComboBox()
            cb.addItem("Halten", "hold")
            cb.addItem("Umschalten", "toggle")
            cb.setCurrentIndex(0 if current == "hold" else 1)
            return cb

        self._hk_transcription    = HotkeyEdit(self.settings.hotkeys.transcription)
        self._hk_text_improvement = HotkeyEdit(self.settings.hotkeys.text_improvement)
        self._hk_dampf_ablassen   = HotkeyEdit(self.settings.hotkeys.dampf_ablassen)
        self._hk_emoji_text       = HotkeyEdit(self.settings.hotkeys.emoji_text)
        self._hk_correction       = HotkeyEdit(self.settings.hotkeys.correction)

        self._hk_transcription_mode    = _mode_combo(self.settings.hotkeys.transcription_mode)
        self._hk_text_improvement_mode = _mode_combo(self.settings.hotkeys.text_improvement_mode)
        self._hk_dampf_ablassen_mode   = _mode_combo(self.settings.hotkeys.dampf_ablassen_mode)
        self._hk_emoji_text_mode       = _mode_combo(self.settings.hotkeys.emoji_text_mode)

        box = QGroupBox("Tastenkürzel")
        grid = QGridLayout(box)
        grid.setHorizontalSpacing(12)
        grid.setVerticalSpacing(10)
        grid.setColumnStretch(1, 3)
        grid.setColumnStretch(2, 2)

        for col, text in enumerate(("Workflow", "Tastenkürzel", "Modus")):
            head = QLabel(text)
            set_prop(head, "role", "label")
            grid.addWidget(head, 0, col)

        rows = [
            ("🎙  BlitzVox",        "kurze Texte",
             self._hk_transcription, self._hk_transcription_mode),
            ("✨  BlitzVox+",       "lange Texte / freies Sprechen",
             self._hk_text_improvement, self._hk_text_improvement_mode),
            ("🔥  BlitzVox $%&!",   "Frust umformulieren",
             self._hk_dampf_ablassen, self._hk_dampf_ablassen_mode),
            ("😊  BlitzVox :)",     "mit Emojis",
             self._hk_emoji_text, self._hk_emoji_text_mode),
            ("✏  Korrigieren",       "Fenster für letzten Text öffnen",
             self._hk_correction, None),
        ]
        for r, (name, desc, key_field, mode_combo) in enumerate(rows, start=1):
            label = QLabel(name)
            label.setToolTip(desc)
            grid.addWidget(label, r, 0)
            grid.addWidget(key_field, r, 1)
            if mode_combo is not None:
                grid.addWidget(mode_combo, r, 2)
            else:
                dash = QLabel("—")
                set_prop(dash, "role", "hint")
                grid.addWidget(dash, r, 2)

        help_text = QLabel(
            "<b>Hotkeys ändern:</b> Ins Feld klicken und die gewünschte Kombination drücken, "
            "z.&nbsp;B. Strg+Alt+W (wird als ctrl+alt+w angezeigt). "
            "Entf oder Rücktaste leert das Feld.<br>"
            "<b>Halten:</b> Kombination gedrückt halten, solange du sprichst. "
            "Beim Loslassen endet die Aufnahme und der Text wird verarbeitet.<br>"
            "<b>Umschalten:</b> Einmal drücken startet die Aufnahme, "
            "nochmal drücken stoppt sie.<br>"
            "Änderungen gelten nach „Speichern“."
        )
        set_prop(help_text, "role", "hint")
        help_text.setWordWrap(True)
        help_text.setTextFormat(Qt.TextFormat.RichText)
        grid.addWidget(help_text, len(rows) + 1, 0, 1, 3)

        # Tabelle ausgrauen, wenn Hotkeys deaktiviert sind
        box.setEnabled(self._hk_enabled.isChecked())
        self._hk_enabled.toggled.connect(box.setEnabled)

        layout.addWidget(box)
        layout.addStretch()
        scroll.setWidget(w)
        return scroll

    # ── Tab: Korrekturen ─────────────────────────────────────────────────────

    def _tab_corrections(self) -> QWidget:
        w = QWidget()
        layout = QVBoxLayout(w)
        layout.setContentsMargins(20, 14, 20, 20)
        layout.setSpacing(12)

        # ─ Satzzeichen ─
        punct_grp = QGroupBox("Satzzeichen")
        pg = QVBoxLayout(punct_grp)
        self._spoken_punct = QCheckBox(
            "Satzzeichen per Sprachmodell setzen (inkl. diktierter Befehle)")
        self._spoken_punct.setChecked(self.settings.spoken_punctuation)
        pg.addWidget(self._spoken_punct)
        pnote = QLabel(
            "Befehle: Komma, Punkt, Fragezeichen, Ausrufezeichen, Doppelpunkt, "
            "Semikolon, Bindestrich, Gedankenstrich, neue Zeile, neuer Absatz, "
            "Klammer auf/zu, Anführungszeichen auf/zu.\n"
            "Bei „BlitzVox“ (reine Transkription) wird dafür bei jedem Diktat das "
            "Sprachmodell aus „BlitzVox+“ aufgerufen. Ist es nicht erreichbar, "
            "greifen einfache Regeln."
        )
        pnote.setWordWrap(True)
        set_prop(pnote, "role", "hint")
        pg.addWidget(pnote)
        layout.addWidget(punct_grp)

        # ─ Gelernte Korrekturen ─
        corr_grp = QGroupBox("Gelernte Korrekturen")
        cg = QVBoxLayout(corr_grp)
        cnote = QLabel(
            "Werden nach der Erkennung ersetzt; die richtige Schreibweise geht "
            "außerdem als Hinweis an die Spracherkennung. Neue Einträge entstehen "
            "über „Letzten Text korrigieren …“ im Tray-Menü oder hier. "
            "Änderungen in dieser Liste werden sofort gespeichert."
        )
        cnote.setWordWrap(True)
        set_prop(cnote, "role", "hint")
        cg.addWidget(cnote)

        self._corr_table = QTableWidget(0, 2)
        self._corr_table.setHorizontalHeaderLabels(["Erkannt (falsch)", "Ersetzen durch"])
        self._corr_table.horizontalHeader().setSectionResizeMode(
            QHeaderView.ResizeMode.Stretch)
        self._corr_table.verticalHeader().setVisible(False)
        self._corr_table.setEditTriggers(QAbstractItemView.EditTrigger.NoEditTriggers)
        self._corr_table.setSelectionBehavior(QAbstractItemView.SelectionBehavior.SelectRows)
        self._corr_table.setMinimumHeight(180)
        cg.addWidget(self._corr_table)

        add_row = QHBoxLayout()
        self._corr_wrong = QLineEdit()
        self._corr_wrong.setPlaceholderText("falsch, z. B. Proxmocks")
        self._corr_right = QLineEdit()
        self._corr_right.setPlaceholderText("richtig, z. B. Proxmox")
        add_btn = QPushButton("Hinzufügen")
        set_prop(add_btn, "variant", "primary")
        add_btn.clicked.connect(self._corr_add)
        add_row.addWidget(self._corr_wrong)
        add_row.addWidget(self._corr_right)
        add_row.addWidget(add_btn)
        cg.addLayout(add_row)

        del_btn = QPushButton("Ausgewählte löschen")
        del_btn.clicked.connect(self._corr_delete)
        cg.addWidget(del_btn, alignment=Qt.AlignmentFlag.AlignRight)

        layout.addWidget(corr_grp)
        layout.addStretch()

        self._corr_reload()
        return w

    def _corr_reload(self) -> None:
        from ..services import corrections
        reps = corrections.load()["replacements"]
        self._corr_table.setRowCount(0)
        for wrong in sorted(reps, key=str.lower):
            row = self._corr_table.rowCount()
            self._corr_table.insertRow(row)
            self._corr_table.setItem(row, 0, QTableWidgetItem(wrong))
            self._corr_table.setItem(row, 1, QTableWidgetItem(reps[wrong]))

    def _corr_add(self) -> None:
        from ..services import corrections
        wrong = self._corr_wrong.text().strip()
        right = self._corr_right.text().strip()
        if not wrong or not right:
            return
        if wrong.lower() == right.lower():
            QMessageBox.information(
                self, "BlitzVox",
                "Falsch und richtig unterscheiden sich nur in der Groß-/Kleinschreibung. "
                "Das regelt das Sprachmodell, dafür ist kein Eintrag nötig.")
            return
        corrections.add_replacement(wrong, right)
        self._corr_wrong.clear()
        self._corr_right.clear()
        self._corr_reload()

    def _corr_delete(self) -> None:
        from ..services import corrections
        rows = sorted({i.row() for i in self._corr_table.selectedIndexes()})
        for row in rows:
            item = self._corr_table.item(row, 0)
            if item:
                corrections.remove_replacement(item.text())
        self._corr_reload()

    # ── Tab: Erweitert ───────────────────────────────────────────────────────

    def _tab_advanced(self) -> QWidget:
        w = QWidget()
        layout = QVBoxLayout(w)
        layout.setContentsMargins(20, 14, 20, 20)
        layout.setSpacing(12)

        self._auto_paste = QCheckBox("Text automatisch in fokussiertes Fenster einfügen")
        self._auto_paste.setChecked(self.settings.auto_paste)
        layout.addWidget(self._auto_paste)

        note = QLabel(
            "Auto-Paste benötigt xdotool (Linux X11) oder ydotool (Wayland) "
            "bzw. pyautogui (Windows).\n"
            "Ohne Auto-Paste landet der Text nur in der Zwischenablage."
        )
        set_prop(note, "role", "hint")
        note.setWordWrap(True)
        layout.addWidget(note)

        self._launch_at_login = QCheckBox("Mit Windows starten" if platform.system() == "Windows" else "Beim Anmelden starten")
        self._launch_at_login.setChecked(self.settings.launch_at_login)
        layout.addWidget(self._launch_at_login)

        # ─ Mikrofon-Auswahl ─
        mic_grp = QGroupBox("🎤  Mikrofon")
        mf = QFormLayout(mic_grp)
        self._mic_combo = NoWheelComboBox()
        self._mic_combo.addItem("System-Standard", None)

        try:
            from ..services.recorder import list_input_devices
            devices = list_input_devices()
            current = self.settings.audio_input_device
            selected_idx = 0
            for i, d in enumerate(devices):
                label = d["name"]
                if d.get("default"):
                    label += "  (Default)"
                self._mic_combo.addItem(label, d["index"])
                if current is not None and d["index"] == current:
                    selected_idx = i + 1
            self._mic_combo.setCurrentIndex(selected_idx)
        except Exception as e:
            self._mic_combo.addItem(f"Fehler: {e}", None)

        mf.addRow("Aufnahmegerät:", self._mic_combo)
        layout.addWidget(mic_grp)

        version_grp = QGroupBox("ℹ️  Über BlitzVox")
        vf = QFormLayout(version_grp)
        version_label = QLabel(f"Version {__version__}")
        set_prop(version_label, "role", "strong")
        vf.addRow("Aktuelle Version:", version_label)
        layout.addWidget(version_grp)

        layout.addStretch()
        return w

    # ── OpenRouter: Modellliste beim Öffnen laden ────────────────────────────

    def showEvent(self, event):
        super().showEvent(event)
        self._start_openrouter_load()

    def _start_openrouter_load(self) -> None:
        if self._or_loading:
            return
        self._or_loading = True
        self._or_status.setText("Modelle werden geladen …")
        threading.Thread(
            target=lambda: self._or_loader.loaded.emit(fetch_openrouter_models()),
            daemon=True,
        ).start()
        threading.Thread(
            target=lambda: self._or_loader.transcription_loaded.emit(fetch_openrouter_transcription_models()),
            daemon=True,
        ).start()

    def _on_openrouter_models(self, ids: list) -> None:
        self._or_loading = False
        current = self._or_model.currentText().strip()
        self._or_model.blockSignals(True)
        self._or_model.clear()
        self._or_model.addItems(ids)
        self._or_model.setEditText(current)
        self._or_model.blockSignals(False)
        if ids:
            self._or_status.setText(f"{len(ids)} Modelle geladen. Auswahl oder eigene ID eintragen.")
        else:
            self._or_status.setText("Keine Modellliste (offline oder kein Key). Modell-ID von Hand eintragen.")

    # ── Save ─────────────────────────────────────────────────────────────────

    def _save(self) -> None:
        s = self.settings

        # Save API keys from fields
        for key_name in [KEY_OPENAI, KEY_GROQ, KEY_ANTHROPIC, KEY_ASSEMBLYAI, KEY_CUSTOM_STT, KEY_CUSTOM_LLM, KEY_OPENROUTER]:
            field = self.findChild(QLineEdit, f"key_{key_name}")
            if field:
                val = field.text().strip()
                if val:
                    set_api_key(key_name, val)

        # Ollama
        s.ollama_base_url  = self._ollama_url.text().strip()
        s.ollama_llm_model = self._ollama_model.text().strip()

        # Custom endpoints
        s.custom_stt_url = self._custom_stt_url.text().strip()
        s.custom_llm_url = self._custom_llm_url.text().strip()

        # OpenRouter
        s.openrouter_llm_model = self._or_model.currentText().strip()

        # Transcription
        s.transcription_model.stt_provider = self._tr_stt_provider.currentData()
        s.transcription_model.stt_model    = self._tr_stt_model.currentText().strip()
        s.transcription_model.language     = self._tr_language.text().strip()

        # BlitzVox+
        s.text_improvement.model.stt_provider = self._ti_stt_provider.currentData()
        s.text_improvement.model.stt_model    = self._ti_stt_model.currentText().strip()
        s.text_improvement.model.llm_provider = self._ti_llm_provider.currentData()
        s.text_improvement.model.llm_model    = self._ti_llm_model.currentText().strip()
        s.text_improvement.tone               = self._ti_tone.currentData()
        s.text_improvement.context            = self._ti_context.text().strip()
        s.text_improvement.system_prompt      = self._ti_prompt.toPlainText().strip()

        # Dampf ablassen
        s.dampf_ablassen.model.stt_provider = self._da_stt_provider.currentData()
        s.dampf_ablassen.model.stt_model    = self._da_stt_model.currentText().strip()
        s.dampf_ablassen.model.llm_provider = self._da_llm_provider.currentData()
        s.dampf_ablassen.model.llm_model    = self._da_llm_model.currentText().strip()
        s.dampf_ablassen.system_prompt      = self._da_prompt.toPlainText().strip()

        # Emoji
        s.emoji.model.stt_provider = self._em_stt_provider.currentData()
        s.emoji.model.stt_model    = self._em_stt_model.currentText().strip()
        s.emoji.model.llm_provider = self._em_llm_provider.currentData()
        s.emoji.model.llm_model    = self._em_llm_model.currentText().strip()
        s.emoji.density            = self._em_density.currentData()

        # Hotkeys
        s.hotkeys.transcription    = self._hk_transcription.text().strip()
        s.hotkeys.text_improvement = self._hk_text_improvement.text().strip()
        s.hotkeys.dampf_ablassen   = self._hk_dampf_ablassen.text().strip()
        s.hotkeys.emoji_text       = self._hk_emoji_text.text().strip()
        s.hotkeys.transcription_mode    = self._hk_transcription_mode.currentData()
        s.hotkeys.text_improvement_mode = self._hk_text_improvement_mode.currentData()
        s.hotkeys.dampf_ablassen_mode   = self._hk_dampf_ablassen_mode.currentData()
        s.hotkeys.emoji_text_mode       = self._hk_emoji_text_mode.currentData()
        s.hotkeys.enabled               = self._hk_enabled.isChecked()
        s.hotkeys.correction            = self._hk_correction.text().strip()

        # Advanced
        s.auto_paste              = self._auto_paste.isChecked()
        s.spoken_punctuation      = self._spoken_punct.isChecked()

        # Autostart: Registry-Eintrag setzen oder entfernen
        s.launch_at_login = self._launch_at_login.isChecked()
        try:
            from ..services.autostart import set_enabled
            set_enabled(s.launch_at_login)
        except Exception as e:
            from PyQt6.QtWidgets import QMessageBox
            QMessageBox.warning(
                self, "BlitzVox",
                f"Der Autostart-Eintrag konnte nicht geändert werden:\n{e}",
            )
        s.audio_input_device      = self._mic_combo.currentData()

        save_settings(s)
        self.settings_saved.emit(s)
        self.close()
