"""
Zentrales Erscheinungsbild von BlitzVox nach dem DAWASTO-Designsystem.

Grundsätze (aus dem Designsystem übernommen):
  - Off-White-Grund (canvas), weiße Karten mit feiner Linie, warmes Fast-Schwarz als Text.
  - Genau eine Aktionsfarbe: die dunkle Ink-Pill (primary). Keine bunten Buttons.
  - Pill-Geometrie für Buttons, weiche Radien für Karten.
  - Immer helles Design, unabhängig vom Dunkelmodus des Systems.
  - Systemschriften (keine eingebetteten Schriftdateien): Serifenschrift für Überschriften,
    serifenlose Systemschrift für alles andere.

Verwendung:
  apply_theme(app)                      einmal in main.py
  set_prop(button, "variant", "primary")  Rolle eines Widgets setzen/ändern
"""

from __future__ import annotations

import os
import tempfile

from PyQt6.QtCore import Qt, QPointF
from PyQt6.QtGui import (
    QColor, QFont, QFontDatabase, QPainter, QPalette, QPen, QPixmap, QPainterPath,
)
from PyQt6.QtWidgets import QApplication, QWidget


# ── Design-Tokens (DAWASTO, Theme „Hell“) ─────────────────────────────────────

T = {
    "primary":         "#292524",
    "primary-active":  "#0c0a09",
    "canvas":          "#f5f5f5",
    "surface-card":    "#ffffff",
    "surface-strong":  "#f0efed",
    "surface-badge":   "#e9e7e4",
    "hairline":        "#e7e5e4",
    "hairline-strong": "#d6d3d1",
    "border-control":  "#8a847f",
    "ink":             "#0c0a09",
    "body":            "#4e4e4e",
    "body-strong":     "#292524",
    "muted":           "#716b64",
    "muted-soft":      "#a8a29e",
    "on-primary":      "#ffffff",
    "success":         "#15803d",
    "error":           "#c81e1e",
}

# Bevorzugte Systemschriften, die erste installierte wird genommen.
_SANS_CANDIDATES = ["Inter", "Segoe UI", "Noto Sans", "Cantarell", "Ubuntu", "DejaVu Sans"]
_SERIF_CANDIDATES = [
    "Cormorant Garamond", "Georgia", "Noto Serif", "Liberation Serif",
    "DejaVu Serif", "Times New Roman",
]


def _first_installed(candidates: list[str], fallback: str) -> str:
    installed = set(QFontDatabase.families())
    for name in candidates:
        if name in installed:
            return name
    return fallback


# ── Kleine Grafiken für Häkchen und Pfeil (zur Laufzeit gezeichnet) ───────────
# Qt-Stylesheets brauchen dafür Bilddateien. Sie werden als PNG in einen
# temporären Ordner geschrieben, so braucht der Build keine zusätzlichen Dateien.

def _icon_dir() -> str:
    path = os.path.join(tempfile.gettempdir(), "blitzvox-theme")
    os.makedirs(path, exist_ok=True)
    return path


def _draw_check(path: str, color: str) -> None:
    pix = QPixmap(36, 36)            # doppelte Größe für scharfe Darstellung
    pix.fill(Qt.GlobalColor.transparent)
    p = QPainter(pix)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    pen = QPen(QColor(color), 4.0)
    pen.setCapStyle(Qt.PenCapStyle.RoundCap)
    pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
    p.setPen(pen)
    line = QPainterPath(QPointF(9, 18.5))
    line.lineTo(15.5, 25)
    line.lineTo(27, 11.5)
    p.drawPath(line)
    p.end()
    pix.save(path, "PNG")


def _draw_chevron(path: str, color: str) -> None:
    pix = QPixmap(24, 24)
    pix.fill(Qt.GlobalColor.transparent)
    p = QPainter(pix)
    p.setRenderHint(QPainter.RenderHint.Antialiasing)
    pen = QPen(QColor(color), 2.4)
    pen.setCapStyle(Qt.PenCapStyle.RoundCap)
    pen.setJoinStyle(Qt.PenJoinStyle.RoundJoin)
    p.setPen(pen)
    line = QPainterPath(QPointF(6, 9))
    line.lineTo(12, 15)
    line.lineTo(18, 9)
    p.drawPath(line)
    p.end()
    pix.save(path, "PNG")


def _make_icons() -> dict[str, str]:
    d = _icon_dir()
    icons = {
        "check":          os.path.join(d, "check-on-ink.png"),
        "chevron":        os.path.join(d, "chevron-ink.png"),
        "chevron-muted":  os.path.join(d, "chevron-muted.png"),
    }
    _draw_check(icons["check"], T["on-primary"])
    _draw_chevron(icons["chevron"], T["ink"])
    _draw_chevron(icons["chevron-muted"], T["muted-soft"])
    # Qt-Stylesheets erwarten Schrägstriche, auch unter Windows
    return {k: v.replace("\\", "/") for k, v in icons.items()}


# ── Stylesheet ────────────────────────────────────────────────────────────────

def _stylesheet(sans: str, serif: str, icons: dict[str, str]) -> str:
    t = T
    return f"""
/* Grundschrift und Textfarbe */
QWidget {{
    font-family: "{sans}";
    font-size: 14px;
    color: {t['body-strong']};
}}

QToolTip {{
    background: {t['primary']};
    color: {t['on-primary']};
    border: none;
    padding: 6px 10px;
    border-radius: 6px;
}}

/* ── Textrollen ─────────────────────────────────────────── */
QLabel[role="display"] {{
    font-family: "{serif}";
    font-size: 26px;
    font-weight: 300;
    color: {t['ink']};
}}
QLabel[role="display-sm"] {{
    font-family: "{serif}";
    font-size: 22px;
    font-weight: 300;
    color: {t['ink']};
}}
QLabel[role="hint"] {{
    color: {t['muted']};
    font-size: 13px;
}}
QLabel[role="lead"] {{
    color: {t['body-strong']};
    font-size: 13px;
    font-weight: 500;
}}
QLabel[role="label"] {{
    color: {t['ink']};
    font-size: 13px;
    font-weight: 600;
}}
QLabel[role="status"] {{
    color: {t['body']};
    font-size: 14px;
}}
QLabel[role="success"] {{
    color: {t['success']};
    font-size: 14px;
    font-weight: 500;
}}
QLabel[role="error"] {{
    color: {t['error']};
    font-size: 14px;
    font-weight: 500;
}}
QLabel[role="strong"] {{
    color: {t['ink']};
    font-weight: 600;
}}
QLabel[role="plaque"] {{
    background: {t['surface-strong']};
    border-radius: 24px;
    font-size: 22px;
    min-width: 48px; max-width: 48px;
    min-height: 48px; max-height: 48px;
}}

/* ── Karten (Gruppen) ───────────────────────────────────── */
QGroupBox {{
    background: {t['surface-card']};
    border: 1px solid {t['hairline-strong']};
    border-radius: 16px;
    margin-top: 26px;
    padding: 6px 16px 14px 16px;
    font-size: 14px;
    font-weight: 600;
    color: {t['ink']};
}}
QGroupBox::title {{
    subcontrol-origin: margin;
    subcontrol-position: top left;
    left: 6px;
    top: 0px;
    padding: 0 2px 6px 2px;
    color: {t['ink']};
}}
QGroupBox QLabel {{
    font-weight: 400;
}}

/* Trennlinie */
QFrame[frameShape="4"] {{
    border: none;
    background: {t['hairline-strong']};
    max-height: 1px;
    min-height: 1px;
}}

/* ── Eingabefelder ──────────────────────────────────────── */
QLineEdit, QTextEdit, QPlainTextEdit, QComboBox, QSpinBox {{
    background: {t['surface-card']};
    border: 1px solid {t['border-control']};
    border-radius: 8px;
    padding: 6px 10px;
    color: {t['ink']};
    selection-background-color: {t['primary']};
    selection-color: {t['on-primary']};
}}
QLineEdit, QComboBox, QSpinBox {{
    min-height: 22px;
}}
QLineEdit:focus, QTextEdit:focus, QPlainTextEdit:focus,
QComboBox:focus, QSpinBox:focus {{
    border: 1px solid {t['ink']};
}}
QLineEdit:disabled, QTextEdit:disabled, QPlainTextEdit:disabled,
QComboBox:disabled, QSpinBox:disabled {{
    background: {t['surface-strong']};
    color: {t['muted']};
    border-color: {t['hairline-strong']};
}}
QTextEdit[readOnly="true"] {{
    border-color: {t['hairline-strong']};
}}

QComboBox {{
    padding-right: 30px;
}}
QComboBox::drop-down {{
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 28px;
    border: none;
    background: transparent;
}}
QComboBox::down-arrow {{
    image: url("{icons['chevron']}");
    width: 12px;
    height: 12px;
}}
QComboBox::down-arrow:disabled {{
    image: url("{icons['chevron-muted']}");
}}
QComboBox QAbstractItemView {{
    background: {t['surface-card']};
    border: 1px solid {t['hairline-strong']};
    padding: 4px;
    outline: none;
    selection-background-color: {t['surface-strong']};
    selection-color: {t['ink']};
}}

/* ── Checkboxen ─────────────────────────────────────────── */
QCheckBox {{
    spacing: 10px;
    color: {t['body-strong']};
    font-weight: 400;
}}
QCheckBox::indicator {{
    width: 18px;
    height: 18px;
    border: 1px solid {t['border-control']};
    border-radius: 5px;
    background: {t['surface-card']};
}}
QCheckBox::indicator:hover {{
    border-color: {t['ink']};
}}
QCheckBox::indicator:checked {{
    background: {t['primary']};
    border-color: {t['primary']};
    image: url("{icons['check']}");
}}
QCheckBox::indicator:disabled {{
    background: {t['surface-strong']};
    border-color: {t['hairline-strong']};
}}

/* ── Buttons: Pill-Form ─────────────────────────────────── */
/* Standard = Outline-Pill */
QPushButton {{
    background: {t['surface-card']};
    color: {t['ink']};
    border: 1px solid {t['border-control']};
    border-radius: 18px;
    padding: 0 18px;
    min-height: 34px;
    font-size: 14px;
    font-weight: 500;
}}
QPushButton:hover {{
    border-color: {t['ink']};
    background: {t['surface-strong']};
}}
QPushButton:pressed, QPushButton:checked {{
    background: {t['surface-badge']};
    border-color: {t['ink']};
}}
QPushButton:disabled {{
    background: {t['surface-strong']};
    color: {t['muted-soft']};
    border-color: {t['surface-strong']};
}}
QPushButton:focus {{
    outline: none;
}}

/* Hauptaktion = Ink-Pill */
QPushButton[variant="primary"] {{
    background: {t['primary']};
    color: {t['on-primary']};
    border: 1px solid {t['primary']};
}}
QPushButton[variant="primary"]:hover {{
    background: {t['primary-active']};
    border-color: {t['primary-active']};
}}
QPushButton[variant="primary"]:pressed {{
    background: {t['primary-active']};
}}
QPushButton[variant="primary"]:disabled {{
    background: {t['surface-strong']};
    color: {t['muted-soft']};
    border-color: {t['surface-strong']};
}}

/* Aufnahme läuft = rote Outline-Pill (Status, keine Akzentfarbe) */
QPushButton[variant="stop"] {{
    background: {t['surface-card']};
    color: {t['error']};
    border: 1px solid {t['error']};
}}
QPushButton[variant="stop"]:hover {{
    background: #fbeeee;
}}

/* Textlink-Button */
QPushButton[variant="text"] {{
    background: transparent;
    border: none;
    color: {t['muted']};
    font-weight: 400;
    padding: 0 8px;
}}
QPushButton[variant="text"]:hover {{
    color: {t['ink']};
    background: transparent;
}}

/* Große Pill im Workflow-Fenster */
QPushButton[size="lg"] {{
    min-height: 38px;
    border-radius: 20px;
    padding: 0 22px;
    font-size: 15px;
}}

/* Runder Icon-Button */
QPushButton[variant="icon"] {{
    padding: 0;
    min-width: 34px;
    max-width: 34px;
}}

/* ── Tabs ───────────────────────────────────────────────── */
QTabWidget::pane {{
    border: none;
    border-top: 1px solid {t['hairline-strong']};
    background: {t['canvas']};
    top: -1px;
}}
QTabBar {{
    background: transparent;
}}
QTabBar::tab {{
    background: transparent;
    color: {t['muted']};
    border: none;
    border-bottom: 2px solid transparent;
    padding: 12px 9px 10px 9px;
    margin: 0 1px;
    font-size: 14px;
    font-weight: 500;
}}
QTabBar::tab:hover {{
    color: {t['ink']};
}}
QTabBar::tab:selected {{
    color: {t['ink']};
    border-bottom: 2px solid {t['ink']};
}}

/* ── Scrollbereiche ─────────────────────────────────────── */
QScrollArea {{
    background: transparent;
    border: none;
}}
QScrollArea > QWidget > QWidget {{
    background: transparent;
}}
QScrollBar:vertical {{
    background: transparent;
    width: 10px;
    margin: 4px 2px 4px 0;
}}
QScrollBar::handle:vertical {{
    background: {t['hairline-strong']};
    border-radius: 4px;
    min-height: 32px;
}}
QScrollBar::handle:vertical:hover {{
    background: {t['muted-soft']};
}}
QScrollBar:horizontal {{
    background: transparent;
    height: 10px;
    margin: 0 4px 2px 4px;
}}
QScrollBar::handle:horizontal {{
    background: {t['hairline-strong']};
    border-radius: 4px;
    min-width: 32px;
}}
QScrollBar::add-line, QScrollBar::sub-line {{
    width: 0; height: 0;
}}
QScrollBar::add-page, QScrollBar::sub-page {{
    background: transparent;
}}

/* ── Tabelle ────────────────────────────────────────────── */
QTableWidget, QTableView {{
    background: {t['surface-card']};
    border: 1px solid {t['hairline-strong']};
    border-radius: 0px;
    gridline-color: {t['hairline']};
    selection-background-color: {t['surface-strong']};
    selection-color: {t['ink']};
    outline: none;
}}
QTableWidget::item, QTableView::item {{
    padding: 4px 8px;
}}
QHeaderView::section {{
    background: {t['surface-strong']};
    color: {t['ink']};
    font-size: 13px;
    font-weight: 600;
    border: none;
    border-bottom: 1px solid {t['hairline-strong']};
    padding: 8px;
}}
QTableCornerButton::section {{
    background: {t['surface-strong']};
    border: none;
}}

/* ── Tray-Menü ──────────────────────────────────────────── */
QMenu {{
    background: {t['surface-card']};
    border: 1px solid {t['hairline-strong']};
    padding: 6px;
}}
QMenu::item {{
    padding: 8px 24px 8px 14px;
    border-radius: 6px;
    color: {t['ink']};
}}
QMenu::item:selected {{
    background: {t['surface-strong']};
}}
QMenu::item:disabled {{
    color: {t['muted']};
}}
QMenu::separator {{
    height: 1px;
    background: {t['hairline']};
    margin: 6px 8px;
}}

/* ── Deaktivierte Bereiche ausgrauen (steht am Ende, damit es Vorrang hat) ── */
QLabel:disabled, QCheckBox:disabled, QGroupBox:disabled {{
    color: {t['muted-soft']};
}}
QGroupBox::title:disabled {{
    color: {t['muted-soft']};
}}

/* ── Dialoge ────────────────────────────────────────────── */
QMessageBox QLabel {{
    color: {t['body-strong']};
}}
"""


def _light_palette() -> QPalette:
    t = T
    pal = QPalette()
    roles = {
        QPalette.ColorRole.Window:          t["canvas"],
        QPalette.ColorRole.WindowText:      t["body-strong"],
        QPalette.ColorRole.Base:            t["surface-card"],
        QPalette.ColorRole.AlternateBase:   t["surface-strong"],
        QPalette.ColorRole.Text:            t["ink"],
        QPalette.ColorRole.Button:          t["surface-card"],
        QPalette.ColorRole.ButtonText:      t["ink"],
        QPalette.ColorRole.BrightText:      t["on-primary"],
        QPalette.ColorRole.Highlight:       t["primary"],
        QPalette.ColorRole.HighlightedText: t["on-primary"],
        QPalette.ColorRole.ToolTipBase:     t["primary"],
        QPalette.ColorRole.ToolTipText:     t["on-primary"],
        QPalette.ColorRole.PlaceholderText: t["muted"],
        QPalette.ColorRole.Link:            t["ink"],
        QPalette.ColorRole.Light:           t["surface-card"],
        QPalette.ColorRole.Midlight:        t["surface-strong"],
        QPalette.ColorRole.Mid:             t["hairline-strong"],
        QPalette.ColorRole.Dark:            t["border-control"],
        QPalette.ColorRole.Shadow:          t["muted"],
    }
    for role, color in roles.items():
        pal.setColor(role, QColor(color))
    for role in (QPalette.ColorRole.WindowText, QPalette.ColorRole.Text,
                 QPalette.ColorRole.ButtonText):
        pal.setColor(QPalette.ColorGroup.Disabled, role, QColor(t["muted-soft"]))
    return pal


# ── Öffentliche Funktionen ────────────────────────────────────────────────────

def apply_theme(app: QApplication) -> None:
    """Setzt Stil, Palette, Schrift und Stylesheet für die ganze Anwendung."""
    # Immer hell, auch wenn das System im Dunkelmodus läuft (ab Qt 6.8).
    try:
        app.styleHints().setColorScheme(Qt.ColorScheme.Light)
    except AttributeError:
        pass

    # Fusion ist auf Windows und Linux gleich und hält sich an Stylesheets.
    app.setStyle("Fusion")
    app.setPalette(_light_palette())

    sans = _first_installed(_SANS_CANDIDATES, app.font().family())
    serif = _first_installed(_SERIF_CANDIDATES, sans)
    font = QFont(sans)
    font.setPixelSize(14)
    app.setFont(font)

    app.setStyleSheet(_stylesheet(sans, serif, _make_icons()))


def set_prop(widget: QWidget, name: str, value: str) -> None:
    """Setzt eine Stil-Eigenschaft (z. B. variant, role) und zeichnet das Widget neu."""
    widget.setProperty(name, value)
    style = widget.style()
    style.unpolish(widget)
    style.polish(widget)
    widget.update()
