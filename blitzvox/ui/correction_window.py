"""
Korrekturfenster: letzten diktierten Text korrigieren und daraus lernen.

Ablauf:
  1. Text erscheint im Eingabefeld und kann bearbeitet werden.
  2. Während des Tippens werden Lern-Vorschläge ("falsch -> richtig") angezeigt.
  3. Nur angehakte Vorschläge werden gespeichert (Standard: kein Haken, damit
     Stil-Änderungen wie "dann -> danach" nicht versehentlich gelernt werden).
  4. "Übernehmen" speichert die Haken und kopiert den Text in die Zwischenablage.
"""

from PyQt6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel, QPushButton,
    QPlainTextEdit, QCheckBox, QApplication,
)
from PyQt6.QtCore import Qt, QTimer
from PyQt6.QtGui import QFont

from ..services import corrections
from .theme import set_prop


class CorrectionWindow(QWidget):
    def __init__(self, text: str, parent=None):
        super().__init__(parent)
        self._original = text
        self._checks: dict[tuple[str, str], QCheckBox] = {}

        self.setWindowTitle("BlitzVox – Text korrigieren")
        self.setWindowFlags(
            Qt.WindowType.Window |
            Qt.WindowType.WindowStaysOnTopHint |
            Qt.WindowType.WindowCloseButtonHint
        )
        self.setMinimumWidth(480)
        self._setup_ui()

        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.setInterval(400)
        self._debounce.timeout.connect(self._refresh_suggestions)
        self.editor.textChanged.connect(self._debounce.start)

    # ── UI ────────────────────────────────────────────────────────────────────

    def _setup_ui(self) -> None:
        layout = QVBoxLayout(self)
        layout.setContentsMargins(24, 24, 24, 20)
        layout.setSpacing(12)

        title = QLabel("✏  Letzten Text korrigieren")
        set_prop(title, "role", "display-sm")
        layout.addWidget(title)

        hint = QLabel("Korrigiere falsch erkannte Wörter. BlitzVox schlägt vor, "
                      "was es sich merken kann.")
        hint.setWordWrap(True)
        set_prop(hint, "role", "hint")
        layout.addWidget(hint)

        self.editor = QPlainTextEdit(self._original)
        self.editor.setMinimumHeight(120)
        layout.addWidget(self.editor)

        sugg_title = QLabel("Merken (nur echte Hörfehler anhaken):")
        set_prop(sugg_title, "role", "label")
        layout.addWidget(sugg_title)

        self._sugg_box = QVBoxLayout()
        self._sugg_box.setSpacing(4)
        layout.addLayout(self._sugg_box)

        self._empty_label = QLabel("Noch keine Änderung erkannt.")
        set_prop(self._empty_label, "role", "hint")
        self._sugg_box.addWidget(self._empty_label)

        self.status = QLabel("")
        set_prop(self.status, "role", "success")
        layout.addWidget(self.status)

        btns = QHBoxLayout()
        btns.setSpacing(10)
        self.apply_btn = QPushButton("✅  Übernehmen && kopieren")
        set_prop(self.apply_btn, "variant", "primary")
        self.apply_btn.clicked.connect(self._apply)
        btns.addWidget(self.apply_btn)

        close_btn = QPushButton("Schließen")
        close_btn.clicked.connect(self.close)
        btns.addWidget(close_btn)
        layout.addLayout(btns)

    # ── Vorschläge ────────────────────────────────────────────────────────────

    def _refresh_suggestions(self) -> None:
        pairs = corrections.suggest_from_edit(
            self._original, self.editor.toPlainText())

        # Haken-Zustand bestehender Vorschläge merken
        old_state = {k: cb.isChecked() for k, cb in self._checks.items()}
        for cb in self._checks.values():
            self._sugg_box.removeWidget(cb)
            cb.deleteLater()
        self._checks.clear()

        for wrong, right in pairs:
            cb = QCheckBox(f"„{wrong}“  →  „{right}“")
            cb.setChecked(old_state.get((wrong, right), False))
            self._sugg_box.addWidget(cb)
            self._checks[(wrong, right)] = cb

        self._empty_label.setVisible(not pairs)
        self.adjustSize()

    # ── Übernehmen ────────────────────────────────────────────────────────────

    def _apply(self) -> None:
        self._debounce.stop()
        self._refresh_suggestions()   # sicher auf aktuellem Stand

        learned = [k for k, cb in self._checks.items() if cb.isChecked()]
        for wrong, right in learned:
            corrections.add_replacement(wrong, right)

        text = self.editor.toPlainText()
        QApplication.clipboard().setText(text)
        corrections.set_last(text)
        self._original = text   # weitere Korrekturen bauen hierauf auf
        self._refresh_suggestions()

        n = len(learned)
        self.status.setText(
            f"✅  {n} Korrektur{'en' if n != 1 else ''} gemerkt, Text in Zwischenablage."
            if n else "✅  Text in Zwischenablage (nichts gemerkt)."
        )
