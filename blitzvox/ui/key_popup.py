"""
Kleines Fenster, das einen API-Key vollständig anzeigt.

Ohne Fensterrahmen, nur ein ✕ oben rechts zum Schließen (alternativ Esc).
Der Key bricht innerhalb des Fensters um, damit lange Keys (z. B. von
Anthropic) die Breite nicht sprengen. Er lässt sich markieren und kopieren.
Nach dem Schließen ist der Key wieder verborgen.
"""

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QFontDatabase, QTextOption
from PyQt6.QtWidgets import (
    QApplication, QDialog, QFrame, QHBoxLayout, QLabel, QLineEdit, QPushButton,
    QTextEdit, QVBoxLayout,
)

from .theme import set_prop, T

_WIDTH = 460


class KeyPopup(QDialog):
    def __init__(self, title: str, key: str, parent=None):
        super().__init__(parent)
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)  # runde Ecken
        self.setModal(True)

        panel = QFrame()
        panel.setObjectName("keyPopupPanel")
        panel.setStyleSheet(
            f"QFrame#keyPopupPanel {{ background: {T['surface-card']}; "
            f"border: 1px solid {T['hairline-strong']}; border-radius: 10px; }}"
            "QFrame#keyPopupPanel QLabel { background: transparent; border: none; }"
        )
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.addWidget(panel)

        layout = QVBoxLayout(panel)
        layout.setContentsMargins(18, 10, 10, 16)
        layout.setSpacing(6)

        head = QHBoxLayout()
        title_label = QLabel(title)
        set_prop(title_label, "role", "strong")
        head.addWidget(title_label, 1)
        close_btn = QPushButton("✕")
        set_prop(close_btn, "variant", "text")
        close_btn.setToolTip("Schließen (Esc)")
        close_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        close_btn.clicked.connect(self.close)
        head.addWidget(close_btn, 0, Qt.AlignmentFlag.AlignTop)
        layout.addLayout(head)

        text = QTextEdit()
        text.setReadOnly(True)
        text.setPlainText(key)
        text.setFrameShape(QFrame.Shape.NoFrame)
        text.setStyleSheet("QTextEdit { background: transparent; border: none; padding: 0; }")
        text.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))
        # an jeder Stelle umbrechen: Keys haben keine Leerzeichen
        text.setLineWrapMode(QTextEdit.LineWrapMode.WidgetWidth)
        text.setWordWrapMode(QTextOption.WrapMode.WrapAnywhere)
        text.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        text.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        layout.addWidget(text)

        hint = QLabel("Zum Kopieren markieren und Strg+C drücken.")
        set_prop(hint, "role", "hint")
        layout.addWidget(hint)

        # Höhe des Textfelds an die Zahl der Zeilen anpassen
        text_width = _WIDTH - 18 - 10
        text.setFixedWidth(text_width)
        text.document().setTextWidth(text_width - 4)
        text.setFixedHeight(int(text.document().size().height()) + 6)

        self.setFixedWidth(_WIDTH)
        self.adjustSize()
        if parent is not None:
            win = parent.window()
            geo = win.frameGeometry()
            self.move(geo.center().x() - self.width() // 2,
                      geo.center().y() - self.height() // 2)


def show_key(title: str, key: str, parent=None) -> None:
    """Key im Fenster anzeigen; kehrt zurück, wenn das Fenster geschlossen ist."""
    KeyPopup(title, key, parent).exec()


# ── Hinweisfenster (gleiche Optik, z. B. „Lokale Spracherkennung fehlt“) ──────

_INFO_WIDTH = 540


class InfoPopup(QDialog):
    """Fenster ohne Rahmen mit ✕ oben rechts. `items` ist eine Liste aus
      ("text", "…")              Absatz (Links als <a href> erlaubt)
      ("code", "…")              Befehl zum Kopieren, mit Knopf „Kopieren“
      ("box", "Titel", "…")      Hinweiskasten mit Rahmen und runden Ecken
    """

    def __init__(self, title: str, items: list, parent=None):
        super().__init__(parent)
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setModal(True)

        panel = QFrame()
        panel.setObjectName("infoPopupPanel")
        panel.setStyleSheet(
            f"QFrame#infoPopupPanel {{ background: {T['surface-card']}; "
            f"border: 1px solid {T['hairline-strong']}; border-radius: 10px; }}"
            "QFrame#infoPopupPanel QLabel { background: transparent; border: none; }"
            f"QFrame#infoBox {{ background: {T['canvas']}; "
            f"border: 1px solid {T['hairline-strong']}; border-radius: 10px; }}"
        )
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.addWidget(panel)

        layout = QVBoxLayout(panel)
        layout.setContentsMargins(20, 10, 10, 18)
        layout.setSpacing(10)

        head = QHBoxLayout()
        title_label = QLabel(title)
        set_prop(title_label, "role", "strong")  # einzeilig: Umbruch ergäbe falsche Höhe
        head.addWidget(title_label, 1)
        close_btn = QPushButton("✕")
        set_prop(close_btn, "variant", "text")
        close_btn.setToolTip("Schließen (Esc)")
        close_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        close_btn.clicked.connect(self.close)
        head.addWidget(close_btn, 0, Qt.AlignmentFlag.AlignTop)
        layout.addLayout(head)

        body = QVBoxLayout()
        body.setContentsMargins(0, 0, 10, 0)
        body.setSpacing(10)
        layout.addLayout(body)

        for item in items:
            kind = item[0]
            if kind == "text":
                body.addWidget(self._text(item[1]))
            elif kind == "code":
                body.addLayout(self._code(item[1]))
            elif kind == "box":
                box = QFrame()
                box.setObjectName("infoBox")
                bl = QVBoxLayout(box)
                bl.setContentsMargins(14, 10, 14, 12)
                bl.setSpacing(4)
                head_lbl = QLabel(item[1])
                set_prop(head_lbl, "role", "strong")
                bl.addWidget(head_lbl)
                bl.addWidget(self._text(item[2]))
                body.addWidget(box)

        # Höhe passend zur festen Breite berechnen (umbrechende Texte), sonst
        # entsteht unter der Überschrift eine Lücke
        self.setFixedWidth(_INFO_WIDTH)
        self.layout().activate()
        self.setFixedHeight(self.layout().totalHeightForWidth(_INFO_WIDTH))
        if parent is not None:
            geo = parent.window().frameGeometry()
            self.move(geo.center().x() - self.width() // 2,
                      geo.center().y() - self.height() // 2)

    @staticmethod
    def _text(text: str) -> QLabel:
        lbl = QLabel(text)
        lbl.setWordWrap(True)
        lbl.setTextFormat(Qt.TextFormat.RichText)
        lbl.setOpenExternalLinks(True)
        lbl.setTextInteractionFlags(Qt.TextInteractionFlag.TextBrowserInteraction)
        return lbl

    @staticmethod
    def _code(command: str) -> QHBoxLayout:
        row = QHBoxLayout()
        field = QLineEdit(command)
        field.setReadOnly(True)
        field.setFont(QFontDatabase.systemFont(QFontDatabase.SystemFont.FixedFont))
        field.setCursorPosition(0)
        row.addWidget(field, 1)
        copy_btn = QPushButton("Kopieren")

        def do_copy():
            QApplication.clipboard().setText(command)
            copy_btn.setText("Kopiert ✓")

        copy_btn.clicked.connect(do_copy)
        row.addWidget(copy_btn)
        return row


def show_info(title: str, items: list, parent=None) -> None:
    InfoPopup(title, items, parent).exec()
